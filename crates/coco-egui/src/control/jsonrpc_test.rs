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

#[test]
fn ping_returns_empty_object() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    );
    assert_eq!(out, Some(json!({"jsonrpc": "2.0", "id": 1, "result": {}})));
}

#[test]
fn notification_gets_no_reply() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    assert!(out.is_none());
}

#[test]
fn notification_with_unknown_method_still_gets_no_reply() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "method": "boom"}),
    );
    assert!(out.is_none());
}

#[test]
fn unknown_method_is_method_not_found() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "id": 2, "method": "nope"}),
    )
    .unwrap();
    assert_eq!(out["error"]["code"], json!(METHOD_NOT_FOUND));
}

#[test]
fn dispatches_to_handler() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "id": 3, "method": "echo", "params": {"a": 1}}),
    )
    .unwrap();
    assert_eq!(out["result"], json!({"a": 1}));
}

#[test]
fn handler_error_is_reported() {
    let out = dispatch(
        &mut EchoHandler,
        json!({"jsonrpc": "2.0", "id": 4, "method": "boom"}),
    )
    .unwrap();
    assert_eq!(out["error"]["code"], json!(INTERNAL_ERROR));
    assert_eq!(out["error"]["message"], json!("boom"));
}

#[test]
fn missing_method_is_invalid_request_with_id_preserved() {
    let out = dispatch(&mut EchoHandler, json!({"jsonrpc": "2.0", "id": 5})).unwrap();
    assert_eq!(out["error"]["code"], json!(INVALID_REQUEST));
    assert_eq!(out["id"], json!(5));
}

#[test]
fn missing_method_with_no_id_still_reports_invalid_request() {
    let out = dispatch(&mut EchoHandler, json!({"jsonrpc": "2.0"})).unwrap();
    assert_eq!(out["error"]["code"], json!(INVALID_REQUEST));
    assert_eq!(out["id"], Value::Null);
}
