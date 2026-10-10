use super::vms::tests::{sample_vms, sample_vms_json};
use super::*;

/// `call`/`list` as a protocol 2025-06-18 client sees them.
const STRUCTURED: bool = true;
/// `call`/`list` as an older client sees them.
const TEXT_ONLY: bool = false;

fn call_params(name: &str, arguments: Value) -> Value {
    json!({"name": name, "arguments": arguments})
}

#[test]
fn missing_required_arg_is_invalid_params() {
    let mut mock = MockBackend::new(vec![]);
    let err = call(&mut mock, call_params("start_vm", json!({})), STRUCTURED).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn unknown_tool_is_invalid_params() {
    let mut mock = MockBackend::new(vec![]);
    let err = call(
        &mut mock,
        call_params("no_such_tool", json!({})),
        STRUCTURED,
    )
    .unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn omitted_arguments_defaults_to_empty_object() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Vms(vec![]))]);
    let result = call(&mut mock, json!({"name": "list_vms"}), STRUCTURED).unwrap();
    assert_eq!(result["isError"], json!(false));
}

#[test]
fn backend_error_is_is_error_true() {
    let mut mock = MockBackend::new(vec![Err("no such VM: coco3".into())]);
    let result = call(&mut mock, call_params("list_vms", json!({})), STRUCTURED).unwrap();
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["content"][0]["text"], json!("no such VM: coco3"));
    assert!(result.get(STRUCTURED_CONTENT).is_none());
}

fn screen_reply(text: &str) -> Reply {
    Reply::Screen(ScreenSnapshot {
        lines: vec![text.into()],
        mode: "text 32x16".into(),
        cursor: Some(coco_core::TextCursor { row: 0, col: 5 }),
    })
}

#[test]
fn screenshot_returns_an_image_block() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Screenshot {
        png_base64: "cGljdHVyZQ==".into(),
        width: 640,
        height: 240,
    })]);
    let result = call(&mut mock, call_params("screenshot", json!({})), STRUCTURED).unwrap();
    assert_eq!(result["isError"], json!(false));
    let content = result["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], json!("image"));
    assert_eq!(content[0]["mimeType"], json!("image/png"));
    assert_eq!(content[0]["data"], json!("cGljdHVyZQ=="));
    assert_eq!(content[1]["type"], json!("text"));
    assert_eq!(content[1]["text"], json!("640×240 framebuffer"));
}

#[test]
fn screen_text_reports_the_cursor_after_the_mode() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Screen(ScreenSnapshot {
        lines: vec!["OK".into(), String::new()],
        mode: "video mode: CoCo-compatible text, base=$0400".into(),
        cursor: Some(coco_core::TextCursor { row: 1, col: 0 }),
    }))]);
    let result = call(&mut mock, call_params("screen_text", json!({})), STRUCTURED).unwrap();
    assert_eq!(
        result["content"][0]["text"],
        json!(
            "```\nOK\n\n```\nvideo mode: CoCo-compatible text, base=$0400\n\
             cursor: row 1, column 0 (0-based)"
        )
    );
}

#[test]
fn screen_text_without_a_cursor_says_so() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Screen(ScreenSnapshot {
        lines: vec!["<no text buffer>".into()],
        mode: "video mode: CoCo-compatible graphics (PMODE), base=$0E00".into(),
        cursor: None,
    }))]);
    let result = call(&mut mock, call_params("screen_text", json!({})), STRUCTURED).unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.ends_with("\ncursor: unknown (graphics mode, or BASIC is not driving this screen)")
    );
}

#[test]
fn every_listed_tool_round_trips_through_the_mock() {
    let cases: Vec<(&str, Value, Vec<Reply>)> = vec![
        ("list_vms", json!({}), vec![Reply::Vms(vec![])]),
        ("start_vm", json!({"vm": "coco3"}), vec![Reply::Done]),
        ("stop_vm", json!({"vm": "coco3"}), vec![Reply::Done]),
        ("suspend_vm", json!({"vm": "coco3"}), vec![Reply::Done]),
        ("screen_text", json!({}), vec![screen_reply("HELLO")]),
        (
            "screenshot",
            json!({}),
            vec![Reply::Screenshot {
                png_base64: "YQ==".into(),
                width: 640,
                height: 240,
            }],
        ),
        ("type_text", json!({"text": "HI\n"}), vec![Reply::Done]),
        (
            "enter_basic",
            json!({"listing": "10 PRINT 1"}),
            vec![Reply::Done, screen_reply("OK")],
        ),
        ("press_keys", json!({"keys": ["A"]}), vec![Reply::Done]),
        ("joystick", json!({"stick": "left"}), vec![Reply::Done]),
        (
            "insert_disk",
            json!({"drive": 0, "path": "/tmp/x.dsk"}),
            vec![Reply::Done],
        ),
        ("eject_disk", json!({"drive": 0}), vec![Reply::Done]),
        ("reset", json!({}), vec![Reply::Done]),
        ("set_running", json!({"running": true}), vec![Reply::Done]),
        ("wait", json!({"fields": 30}), vec![Reply::Done]),
        (
            "wait_for_text",
            json!({"pattern": "OK", "timeout_fields": 60}),
            vec![screen_reply("OK")],
        ),
        (
            "peek",
            json!({"addr": 0, "len": 4}),
            vec![Reply::Bytes(vec![1, 2, 3, 4])],
        ),
        (
            "poke",
            json!({"addr": 0, "bytes": [1, 2, 3]}),
            vec![Reply::Done],
        ),
        (
            "load_binary",
            json!({"bytes": "AQID", "address": 0x1000}),
            vec![Reply::Done],
        ),
        ("save_state", json!({"slot": 1}), vec![Reply::Done]),
        (
            "load_state",
            json!({"path": "/tmp/state.ccstate"}),
            vec![Reply::Done],
        ),
    ];
    assert_eq!(
        cases.len(),
        tool_defs::definitions(true).len(),
        "every tool must be covered here"
    );

    for (name, args, replies) in cases {
        let mut mock = MockBackend::new(replies.into_iter().map(Ok).collect());
        let result = call(&mut mock, call_params(name, args), STRUCTURED)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            result["isError"],
            json!(false),
            "{name} should have dispatched cleanly"
        );
    }
}

#[test]
fn wait_for_text_builds_a_literal_matcher_and_formats_cursor() {
    let mut mock = MockBackend::new(vec![Ok(screen_reply("READY"))]);
    let result = call(
        &mut mock,
        call_params(
            "wait_for_text",
            json!({"pattern": "READY", "timeout_fields": 120}),
        ),
        STRUCTURED,
    )
    .unwrap();

    assert_eq!(result["isError"], json!(false));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("READY"));
    assert!(text.contains("cursor: row 0, column 5 (0-based)"));
    assert_eq!(
        result[STRUCTURED_CONTENT]["cursor"],
        json!({"row": 0, "col": 5})
    );
    assert!(matches!(
        mock.calls[0].action,
        Action::WaitForText {
            matcher: TextMatcher::Literal(ref pattern),
            timeout_fields: 120,
            fast_forward: false,
        } if pattern == "READY"
    ));
}

#[test]
fn wait_for_text_rejects_an_invalid_or_overlong_regex_before_backend_call() {
    let mut mock = MockBackend::new(vec![]);
    let invalid = call(
        &mut mock,
        call_params(
            "wait_for_text",
            json!({"pattern": "(", "regex": true, "timeout_fields": 1}),
        ),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(invalid["isError"], json!(true));
    assert!(mock.calls.is_empty());

    let overlong = "x".repeat(crate::control::MAX_WAIT_PATTERN_CHARS + 1);
    let result = call(
        &mut mock,
        call_params(
            "wait_for_text",
            json!({"pattern": overlong, "timeout_fields": 1}),
        ),
        STRUCTURED,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert!(mock.calls.is_empty());
}

#[test]
fn wait_for_text_timeout_error_formats_the_last_screen() {
    let screen = ScreenSnapshot {
        lines: vec!["STILL LOADING".into()],
        mode: "video mode: GIME hi-res text".into(),
        cursor: None,
    };
    let error = ControlError::with_screen("timed out waiting for screen text", screen);
    let mut mock = MockBackend::new(vec![Err(error)]);

    let result = call(
        &mut mock,
        call_params(
            "wait_for_text",
            json!({"pattern": "OK", "timeout_fields": 60}),
        ),
        STRUCTURED,
    )
    .unwrap();

    assert_eq!(result["isError"], json!(true));
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("timed out"));
    assert!(text.contains("STILL LOADING"));
    assert!(text.contains("GIME hi-res text"));
    assert!(text.contains("cursor: unknown"));
    assert!(result.get(STRUCTURED_CONTENT).is_none());
}

/// Checks `value` against the JSON Schema subset `tool_defs` uses (`type`,
/// including a list of types, `properties`, `required`, `items`, `enum`,
/// `minimum`, `maximum`, `maxItems`), and rejects properties the schema
/// doesn't name.
fn assert_conforms(value: &Value, schema: &Value, path: &str) {
    match &schema["type"] {
        Value::String(kind) => assert_conforms_to_type(value, kind, schema, path),
        Value::Array(kinds) => {
            let kind = kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|&kind| json_type_matches(value, kind))
                .unwrap_or_else(|| panic!("{path}: {value} matches none of {kinds:?}"));
            assert_conforms_to_type(value, kind, schema, path);
        }
        other => panic!("{path}: schema type {other} not handled here"),
    }
}

fn json_type_matches(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        other => panic!("schema type {other} not handled here"),
    }
}

fn assert_conforms_to_type(value: &Value, kind: &str, schema: &Value, path: &str) {
    assert!(json_type_matches(value, kind), "{path}: not a {kind}");
    match kind {
        "object" => {
            let object = value.as_object().unwrap();
            for key in schema["required"].as_array().into_iter().flatten() {
                let key = key.as_str().unwrap();
                assert!(object.contains_key(key), "{path}: missing {key}");
            }
            for (key, item) in object {
                let property = &schema["properties"][key];
                assert!(!property.is_null(), "{path}.{key}: not in the schema");
                assert_conforms(item, property, &format!("{path}.{key}"));
            }
        }
        "array" => {
            let items = value.as_array().unwrap();
            if let Some(max) = schema["maxItems"].as_u64() {
                assert!(items.len() as u64 <= max, "{path}: over maxItems");
            }
            for (i, item) in items.iter().enumerate() {
                assert_conforms(item, &schema["items"], &format!("{path}[{i}]"));
            }
        }
        "string" => {
            if let Some(allowed) = schema["enum"].as_array() {
                assert!(allowed.contains(value), "{path}: {value} not in enum");
            }
        }
        "integer" => {
            let n = value.as_i64().unwrap();
            if let Some(min) = schema["minimum"].as_i64() {
                assert!(n >= min, "{path}: {n} below minimum");
            }
            if let Some(max) = schema["maximum"].as_i64() {
                assert!(n <= max, "{path}: {n} above maximum");
            }
        }
        _ => {}
    }
}

fn output_schema(name: &str) -> Value {
    tool_defs::definitions(true)
        .into_iter()
        .find(|d| d["name"] == name)
        .and_then(|mut d| d.get_mut(OUTPUT_SCHEMA).map(Value::take))
        .unwrap_or_else(|| panic!("{name} has no outputSchema"))
}

/// One success case for each tool that declares an `outputSchema`.
fn structured_cases() -> Vec<(&'static str, Value, Reply)> {
    vec![
        ("list_vms", json!({}), Reply::Vms(sample_vms())),
        (
            "screen_text",
            json!({}),
            Reply::Screen(ScreenSnapshot {
                lines: vec!["HELLO".into(), "OK".into()],
                mode: "video mode: CoCo-compatible text, base=$0400".into(),
                cursor: Some(coco_core::TextCursor { row: 2, col: 0 }),
            }),
        ),
        (
            "screen_text",
            json!({}),
            Reply::Screen(ScreenSnapshot {
                lines: vec!["<no text buffer>".into()],
                mode: "video mode: CoCo-compatible graphics (PMODE), base=$0E00".into(),
                cursor: None,
            }),
        ),
        (
            "wait_for_text",
            json!({"pattern": "OK", "timeout_fields": 60}),
            Reply::Screen(ScreenSnapshot {
                lines: vec!["READY".into(), "OK".into()],
                mode: "video mode: GIME hi-res text, base=$000000".into(),
                cursor: Some(coco_core::TextCursor { row: 2, col: 0 }),
            }),
        ),
        (
            "peek",
            json!({"addr": 0xFFFE, "len": 4}),
            Reply::Bytes(vec![0x00, 0x7F, 0x80, 0xFF]),
        ),
        (
            "peek",
            json!({"addr": 0x7FFFE, "len": 2, "physical": true}),
            Reply::Bytes(vec![0x41, 0x42]),
        ),
    ]
}

#[test]
fn structured_content_conforms_to_the_output_schema() {
    for (name, args, reply) in structured_cases() {
        let mut mock = MockBackend::new(vec![Ok(reply)]);
        let result = call(&mut mock, call_params(name, args), STRUCTURED).unwrap();
        let structured = result
            .get(STRUCTURED_CONTENT)
            .unwrap_or_else(|| panic!("{name}: no structuredContent"));
        assert_conforms(structured, &output_schema(name), name);
        assert_eq!(result["content"][0]["type"], json!("text"), "{name}");
    }
}

#[test]
fn structured_content_carries_the_reply_values() {
    let cases = structured_cases();
    let expected = [
        sample_vms_json(),
        json!({
            "lines": ["HELLO", "OK"],
            "mode": "video mode: CoCo-compatible text, base=$0400",
            "cursor": {"row": 2, "col": 0}
        }),
        json!({
            "lines": ["<no text buffer>"],
            "mode": "video mode: CoCo-compatible graphics (PMODE), base=$0E00"
        }),
        json!({
            "lines": ["READY", "OK"],
            "mode": "video mode: GIME hi-res text, base=$000000",
            "cursor": {"row": 2, "col": 0}
        }),
        json!({"addr": 0xFFFE, "physical": false, "bytes": [0x00, 0x7F, 0x80, 0xFF]}),
        json!({"addr": 0x7FFFE, "physical": true, "bytes": [0x41, 0x42]}),
    ];
    for ((name, args, reply), expected) in cases.into_iter().zip(expected) {
        let mut mock = MockBackend::new(vec![Ok(reply)]);
        let result = call(&mut mock, call_params(name, args), STRUCTURED).unwrap();
        assert_eq!(result[STRUCTURED_CONTENT], expected, "{name}");
    }
}

#[test]
fn text_only_clients_get_the_same_text_without_structured_content() {
    for (name, args, reply) in structured_cases() {
        let mut structured_mock = MockBackend::new(vec![Ok(reply.clone())]);
        let structured = call(
            &mut structured_mock,
            call_params(name, args.clone()),
            STRUCTURED,
        )
        .unwrap();
        let mut text_mock = MockBackend::new(vec![Ok(reply)]);
        let text_only = call(&mut text_mock, call_params(name, args), TEXT_ONLY).unwrap();
        assert!(text_only.get(STRUCTURED_CONTENT).is_none(), "{name}");
        assert_eq!(text_only["content"], structured["content"], "{name}");
        assert_eq!(text_only["isError"], json!(false), "{name}");
    }
}

#[test]
fn list_declares_output_schemas_only_for_structured_clients() {
    let has_schema = |tools: &Value| {
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t.get(OUTPUT_SCHEMA).is_some())
    };
    assert!(has_schema(&list(true, STRUCTURED)));
    assert!(!has_schema(&list(true, TEXT_ONLY)));
    assert_eq!(
        list(true, TEXT_ONLY)["tools"].as_array().unwrap().len(),
        tool_defs::definitions(true).len()
    );
}

#[test]
fn wait_passes_fast_forward_through_and_reports_it() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call(
        &mut mock,
        call_params("wait", json!({"fields": 120, "fast_forward": true})),
        STRUCTURED,
    )
    .unwrap();

    assert_eq!(result["isError"], json!(false));
    assert_eq!(
        result["content"][0]["text"],
        json!("Fast-forwarded 120 fields.")
    );
    assert_eq!(
        mock.calls[0].action,
        Action::Wait {
            fields: 120,
            fast_forward: true,
        }
    );
}

#[test]
fn wait_defaults_to_real_time_pacing() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call(
        &mut mock,
        call_params("wait", json!({"fields": 120})),
        STRUCTURED,
    )
    .unwrap();

    assert_eq!(result["content"][0]["text"], json!("Waited 120 fields."));
    assert_eq!(
        mock.calls[0].action,
        Action::Wait {
            fields: 120,
            fast_forward: false,
        }
    );
}

#[test]
fn wait_for_text_passes_fast_forward_through() {
    let mut mock = MockBackend::new(vec![Ok(screen_reply("READY"))]);
    call(
        &mut mock,
        call_params(
            "wait_for_text",
            json!({"pattern": "READY", "timeout_fields": 120, "fast_forward": true}),
        ),
        STRUCTURED,
    )
    .unwrap();

    assert!(matches!(
        mock.calls[0].action,
        Action::WaitForText {
            fast_forward: true,
            ..
        }
    ));
}
