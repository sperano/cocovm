use serde_json::{Value, json};

use super::*;
use crate::control::tools::{MockBackend, call};

/// `call` as a protocol 2025-06-18 client sees it.
const STRUCTURED: bool = true;

fn call_tool(mock: &mut MockBackend, name: &str, arguments: Value) -> Value {
    call(
        mock,
        json!({"name": name, "arguments": arguments}),
        STRUCTURED,
    )
    .unwrap()
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

#[test]
fn hex_dump_pads_a_short_last_line_so_the_ascii_column_aligns() {
    let bytes: Vec<u8> = (0x3C..0x50).collect();
    assert_eq!(
        hex_dump(MemAddr::Logical(0x1000), &bytes),
        "1000: 3C 3D 3E 3F 40 41 42 43 44 45 46 47 48 49 4A 4B  |<=>?@ABCDEFGHIJK|\n\
         1010: 4C 4D 4E 4F                                      |LMNO|"
    );
}

#[test]
fn hex_dump_shows_non_printable_bytes_as_dots() {
    assert_eq!(
        hex_dump(
            MemAddr::Logical(0),
            &[0x00, 0x1F, 0x20, 0x7E, 0x7F, 0x80, 0xFF]
        ),
        "0000: 00 1F 20 7E 7F 80 FF                             |.. ~...|"
    );
}

#[test]
fn hex_dump_wraps_logical_addresses_past_ffff() {
    let dump = hex_dump(MemAddr::Logical(0xFFF8), &[0; 24]);
    let addrs: Vec<&str> = dump.lines().map(|line| &line[..4]).collect();
    assert_eq!(addrs, ["FFF8", "0008"]);
}

#[test]
fn hex_dump_prints_physical_addresses_with_six_digits() {
    let dump = hex_dump(MemAddr::Physical(0x7FFF0), &[0; 32]);
    let addrs: Vec<&str> = dump.lines().map(|line| &line[..6]).collect();
    assert_eq!(addrs, ["07FFF0", "080000"]);
}

#[test]
fn peek_defaults_to_a_logical_address() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Bytes(vec![b'O', b'K']))]);
    let result = call_tool(&mut mock, "peek", json!({"addr": 0x0400, "len": 2}));
    assert_eq!(
        text(&result),
        format!("0400: 4F 4B{}  |OK|", " ".repeat(42))
    );
    assert_eq!(
        mock.calls[0].action,
        Action::Peek {
            addr: MemAddr::Logical(0x0400),
            len: 2
        }
    );
}

#[test]
fn peek_physical_accepts_an_offset_above_ffff() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Bytes(vec![0x12]))]);
    let result = call_tool(
        &mut mock,
        "peek",
        json!({"addr": 0x70000, "len": 1, "physical": true}),
    );
    assert!(text(&result).starts_with("070000: 12 "));
    assert_eq!(
        mock.calls[0].action,
        Action::Peek {
            addr: MemAddr::Physical(0x70000),
            len: 1
        }
    );
}

#[test]
fn logical_address_above_ffff_is_invalid_params() {
    for (name, args) in [
        ("peek", json!({"addr": 0x10000, "len": 1})),
        ("poke", json!({"addr": 0x10000, "bytes": [0]})),
    ] {
        let mut mock = MockBackend::new(vec![]);
        let err = call(
            &mut mock,
            json!({"name": name, "arguments": args}),
            STRUCTURED,
        )
        .unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS, "{name}");
        assert!(err.message.contains("$10000"), "{name}: {}", err.message);
        assert!(mock.calls.is_empty(), "{name} must not reach the backend");
    }
}

#[test]
fn poke_physical_sends_a_physical_address() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(
        &mut mock,
        "poke",
        json!({"addr": 0x12345, "bytes": [1, 2], "physical": true}),
    );
    assert_eq!(text(&result), "Wrote 2 byte(s) at physical $012345.");
    assert_eq!(
        mock.calls[0].action,
        Action::Poke {
            addr: MemAddr::Physical(0x12345),
            bytes: vec![1, 2]
        }
    );
}

#[test]
fn poke_logical_reports_a_four_digit_address() {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(&mut mock, "poke", json!({"addr": 0x400, "bytes": [1]}));
    assert_eq!(text(&result), "Wrote 1 byte(s) at $0400.");
}
