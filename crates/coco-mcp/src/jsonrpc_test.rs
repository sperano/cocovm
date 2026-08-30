use super::*;

struct EchoHandler;

impl Handler for EchoHandler {
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "echo" => Ok(params),
            "boom" => Err(RpcError::new(INTERNAL_ERROR, "boom")),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("no such method: {other}"),
            )),
        }
    }
}

/// Feeds `input` (one JSON-RPC message per line) through [`run`] and parses
/// each response line back into a [`Value`].
fn responses(input: &str) -> Vec<Value> {
    let mut output = Vec::new();
    run(input.as_bytes(), &mut output, &mut EchoHandler).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn ping_returns_empty_object() {
    let out = responses(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
    assert_eq!(out, vec![json!({"jsonrpc": "2.0", "id": 1, "result": {}})]);
}

#[test]
fn notification_gets_no_reply() {
    let out = responses(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert!(out.is_empty());
}

#[test]
fn notification_with_unknown_method_still_gets_no_reply() {
    let out = responses(
        "{\"jsonrpc\":\"2.0\",\"method\":\"boom\"}\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}",
    );
    // Only the second, non-notification line produces a response.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["id"], json!(9));
}

#[test]
fn unknown_method_is_method_not_found() {
    let out = responses(r#"{"jsonrpc":"2.0","id":2,"method":"nope"}"#);
    assert_eq!(out[0]["error"]["code"], json!(METHOD_NOT_FOUND));
}

#[test]
fn dispatches_to_handler() {
    let out = responses(r#"{"jsonrpc":"2.0","id":3,"method":"echo","params":{"a":1}}"#);
    assert_eq!(out[0]["result"], json!({"a": 1}));
}

#[test]
fn handler_error_is_reported() {
    let out = responses(r#"{"jsonrpc":"2.0","id":4,"method":"boom"}"#);
    assert_eq!(out[0]["error"]["code"], json!(INTERNAL_ERROR));
    assert_eq!(out[0]["error"]["message"], json!("boom"));
}

#[test]
fn malformed_json_is_parse_error() {
    let out = responses("not json at all\n");
    assert_eq!(out[0]["error"]["code"], json!(PARSE_ERROR));
    assert_eq!(out[0]["id"], Value::Null);
}

#[test]
fn missing_method_is_invalid_request_with_id_preserved() {
    let out = responses(r#"{"jsonrpc":"2.0","id":5}"#);
    assert_eq!(out[0]["error"]["code"], json!(INVALID_REQUEST));
    assert_eq!(out[0]["id"], json!(5));
}

#[test]
fn blank_lines_are_skipped() {
    let out = responses("\n\n{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"ping\"}\n\n");
    assert_eq!(out.len(), 1);
}
