use super::*;
use crate::control::protocol::VmStatus;

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
fn peek_formats_a_hex_dump_sixteen_bytes_per_line() {
    let bytes: Vec<u8> = (0..20).collect();
    let mut mock = MockBackend::new(vec![Ok(Reply::Bytes(bytes))]);
    let result = call(
        &mut mock,
        call_params("peek", json!({"addr": 0x1000, "len": 20})),
        STRUCTURED,
    )
    .unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        text,
        "1000: 00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F\n1010: 10 11 12 13"
    );
}

#[test]
fn every_listed_tool_round_trips_through_the_mock() {
    let cases: Vec<(&str, Value, Reply)> = vec![
        ("list_vms", json!({}), Reply::Vms(vec![])),
        ("start_vm", json!({"vm": "coco3"}), Reply::Done),
        (
            "screen_text",
            json!({}),
            Reply::Screen {
                lines: vec!["HELLO".into()],
                mode: "text 32x16".into(),
            },
        ),
        (
            "screenshot",
            json!({}),
            Reply::Screenshot {
                png_base64: "YQ==".into(),
                width: 640,
                height: 240,
            },
        ),
        ("type_text", json!({"text": "HI\n"}), Reply::Done),
        ("press_keys", json!({"keys": ["A"]}), Reply::Done),
        ("joystick", json!({"stick": "left"}), Reply::Done),
        (
            "insert_disk",
            json!({"drive": 0, "path": "/tmp/x.dsk"}),
            Reply::Done,
        ),
        ("eject_disk", json!({"drive": 0}), Reply::Done),
        ("reset", json!({}), Reply::Done),
        ("set_running", json!({"running": true}), Reply::Done),
        ("wait", json!({"fields": 30}), Reply::Done),
        (
            "peek",
            json!({"addr": 0, "len": 4}),
            Reply::Bytes(vec![1, 2, 3, 4]),
        ),
        ("poke", json!({"addr": 0, "bytes": [1, 2, 3]}), Reply::Done),
    ];
    assert_eq!(
        cases.len(),
        tool_defs::definitions().len(),
        "every tool must be covered here"
    );

    for (name, args, reply) in cases {
        let mut mock = MockBackend::new(vec![Ok(reply)]);
        let result = call(&mut mock, call_params(name, args), STRUCTURED)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            result["isError"],
            json!(false),
            "{name} should have dispatched cleanly"
        );
    }
}

/// Checks `value` against the JSON Schema subset `tool_defs` uses (`type`,
/// `properties`, `required`, `items`, `enum`, `minimum`, `maximum`,
/// `maxItems`), and rejects properties the schema doesn't name.
fn assert_conforms(value: &Value, schema: &Value, path: &str) {
    match schema["type"].as_str().expect("schema has a type") {
        "object" => {
            let object = value
                .as_object()
                .unwrap_or_else(|| panic!("{path}: not an object"));
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
            let items = value
                .as_array()
                .unwrap_or_else(|| panic!("{path}: not an array"));
            if let Some(max) = schema["maxItems"].as_u64() {
                assert!(items.len() as u64 <= max, "{path}: over maxItems");
            }
            for (i, item) in items.iter().enumerate() {
                assert_conforms(item, &schema["items"], &format!("{path}[{i}]"));
            }
        }
        "string" => {
            assert!(value.is_string(), "{path}: not a string");
            if let Some(allowed) = schema["enum"].as_array() {
                assert!(allowed.contains(value), "{path}: {value} not in enum");
            }
        }
        "integer" => {
            let n = value
                .as_i64()
                .unwrap_or_else(|| panic!("{path}: not an integer"));
            if let Some(min) = schema["minimum"].as_i64() {
                assert!(n >= min, "{path}: {n} below minimum");
            }
            if let Some(max) = schema["maximum"].as_i64() {
                assert!(n <= max, "{path}: {n} above maximum");
            }
        }
        other => panic!("{path}: schema type {other} not handled here"),
    }
}

fn output_schema(name: &str) -> Value {
    tool_defs::definitions()
        .into_iter()
        .find(|d| d["name"] == name)
        .and_then(|mut d| d.get_mut(OUTPUT_SCHEMA).map(Value::take))
        .unwrap_or_else(|| panic!("{name} has no outputSchema"))
}

/// One success case for each tool that declares an `outputSchema`.
fn structured_cases() -> Vec<(&'static str, Value, Reply)> {
    let vms = VmStatus::ALL
        .iter()
        .enumerate()
        .map(|(i, &status)| VmInfo {
            slug: format!("vm{i}"),
            name: format!("VM {i}"),
            status,
        })
        .collect();
    vec![
        ("list_vms", json!({}), Reply::Vms(vms)),
        (
            "screen_text",
            json!({}),
            Reply::Screen {
                lines: vec!["HELLO".into(), "OK".into()],
                mode: "video mode: CoCo-compatible text, base=$0400".into(),
            },
        ),
        (
            "peek",
            json!({"addr": 0xFFFE, "len": 4}),
            Reply::Bytes(vec![0x00, 0x7F, 0x80, 0xFF]),
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
        json!({"vms": [
            {"slug": "vm0", "name": "VM 0", "status": "running"},
            {"slug": "vm1", "name": "VM 1", "status": "suspended"},
            {"slug": "vm2", "name": "VM 2", "status": "powered_off"}
        ]}),
        json!({
            "lines": ["HELLO", "OK"],
            "mode": "video mode: CoCo-compatible text, base=$0400"
        }),
        json!({"addr": 0xFFFE, "bytes": [0x00, 0x7F, 0x80, 0xFF]}),
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
    assert!(has_schema(&list(STRUCTURED)));
    assert!(!has_schema(&list(TEXT_ONLY)));
    assert_eq!(
        list(TEXT_ONLY)["tools"].as_array().unwrap().len(),
        tool_defs::definitions().len()
    );
}
