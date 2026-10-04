use super::*;

fn call_params(name: &str, arguments: Value) -> Value {
    json!({"name": name, "arguments": arguments})
}

#[test]
fn missing_required_arg_is_invalid_params() {
    let mut mock = MockBackend::new(vec![]);
    let err = call(&mut mock, call_params("start_vm", json!({}))).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn unknown_tool_is_invalid_params() {
    let mut mock = MockBackend::new(vec![]);
    let err = call(&mut mock, call_params("no_such_tool", json!({}))).unwrap_err();
    assert_eq!(err.code, INVALID_PARAMS);
}

#[test]
fn omitted_arguments_defaults_to_empty_object() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Vms(vec![]))]);
    let result = call(&mut mock, json!({"name": "list_vms"})).unwrap();
    assert_eq!(result["isError"], json!(false));
}

#[test]
fn backend_error_is_is_error_true() {
    let mut mock = MockBackend::new(vec![Err("no such VM: coco3".into())]);
    let result = call(&mut mock, call_params("list_vms", json!({}))).unwrap();
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["content"][0]["text"], json!("no such VM: coco3"));
}

#[test]
fn screenshot_returns_an_image_block() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Screenshot {
        png_base64: "cGljdHVyZQ==".into(),
        width: 640,
        height: 240,
    })]);
    let result = call(&mut mock, call_params("screenshot", json!({}))).unwrap();
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
        tool_defs::definitions(true).len(),
        "every tool must be covered here"
    );

    for (name, args, reply) in cases {
        let mut mock = MockBackend::new(vec![Ok(reply)]);
        let result =
            call(&mut mock, call_params(name, args)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            result["isError"],
            json!(false),
            "{name} should have dispatched cleanly"
        );
    }
}
