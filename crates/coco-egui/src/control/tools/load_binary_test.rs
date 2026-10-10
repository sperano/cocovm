use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use super::*;
use crate::control::Reply;
use crate::control::tools::{MockBackend, call};

const STRUCTURED: bool = true;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn call_tool(mock: &mut MockBackend, arguments: Value) -> Result<Value, RpcError> {
    call(
        mock,
        json!({"name": "load_binary", "arguments": arguments}),
        STRUCTURED,
    )
}

fn encoded(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

fn decb() -> Vec<u8> {
    vec![
        0x00, 0x00, 0x02, 0x10, 0x00, 0xAA, 0xBB, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xCC, 0xFF, 0x00,
        0x00, 0x20, 0x00,
    ]
}

#[test]
fn inline_decb_is_fully_parsed_and_does_not_execute_by_default() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(&mut mock, json!({"bytes": encoded(&decb())})).unwrap();

    assert_eq!(result["isError"], json!(false));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("2 DECB segment(s)")
    );
    assert_eq!(
        mock.calls[0].action,
        Action::LoadBinary {
            segments: vec![
                DecbSegment {
                    address: 0x1000,
                    bytes: vec![0xAA, 0xBB]
                },
                DecbSegment {
                    address: 0xFFFF,
                    bytes: vec![0xCC]
                }
            ],
            exec_address: None,
        }
    );
}

#[test]
fn decb_exec_uses_the_trailer_address() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(
        &mut mock,
        json!({"vm": "demo", "bytes": encoded(&decb()), "exec": true}),
    )
    .unwrap();

    assert_eq!(mock.calls[0].vm.as_deref(), Some("demo"));
    assert!(matches!(
        mock.calls[0].action,
        Action::LoadBinary {
            exec_address: Some(0x2000),
            ..
        }
    ));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Started execution at $2000")
    );
}

#[test]
fn address_treats_the_source_as_raw_and_executes_at_that_address() {
    let bytes = [0x7E, 0x12, 0x34];
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(
        &mut mock,
        json!({"bytes": encoded(&bytes), "address": 0xFFFE, "exec": true}),
    )
    .unwrap();

    assert_eq!(
        mock.calls[0].action,
        Action::LoadBinary {
            segments: vec![DecbSegment {
                address: 0xFFFE,
                bytes: bytes.to_vec()
            }],
            exec_address: Some(0xFFFE),
        }
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Loaded 3 raw byte(s) at $FFFE.")
    );
}

#[test]
fn path_reads_the_entire_source_without_a_poke_size_limit() {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cocovm-load-binary-{}-{sequence}.bin",
        std::process::id()
    ));
    let bytes = vec![0x5A; crate::control::MAX_POKE_LEN + 1];
    std::fs::write(&path, &bytes).unwrap();
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);

    let result = call_tool(&mut mock, json!({"path": path, "address": 0x4000})).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert_eq!(result["isError"], json!(false));
    let Action::LoadBinary { segments, .. } = &mock.calls[0].action else {
        panic!("expected load_binary action");
    };
    assert_eq!(segments[0].bytes.len(), bytes.len());
}

#[test]
fn exactly_one_source_is_required() {
    for arguments in [json!({}), json!({"path": "x.bin", "bytes": encoded(&[])})] {
        let mut mock = MockBackend::new(vec![]);
        let error = call_tool(&mut mock, arguments).unwrap_err();
        assert_eq!(error.code, INVALID_PARAMS);
        assert!(error.message.contains("exactly one"));
        assert!(mock.calls.is_empty());
    }
}

#[test]
fn malformed_inline_sources_do_not_reach_the_backend() {
    for arguments in [
        json!({"bytes": "not base64", "address": 0x1000}),
        json!({"bytes": encoded(&[0x00, 0x00])}),
    ] {
        let mut mock = MockBackend::new(vec![]);
        let result = call_tool(&mut mock, arguments).unwrap();
        assert_eq!(result["isError"], json!(true));
        assert!(mock.calls.is_empty());
    }
}

#[test]
fn missing_path_is_a_tool_error_and_does_not_reach_the_backend() {
    let mut mock = MockBackend::new(vec![]);
    let result = call_tool(
        &mut mock,
        json!({"path": "/no/such/cocovm-load-binary.bin", "address": 0}),
    )
    .unwrap();

    assert_eq!(result["isError"], json!(true));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("could not read")
    );
    assert!(mock.calls.is_empty());
}
