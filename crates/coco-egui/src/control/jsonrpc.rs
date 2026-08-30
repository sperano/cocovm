//! Minimal JSON-RPC 2.0 message handling. Knows only the protocol-level
//! methods (`ping`, the lifecycle notifications MCP clients send);
//! everything else is handed to a [`Handler`]. No I/O — `control::http`
//! carries the bytes, this module only turns one parsed message into
//! another.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Standard JSON-RPC 2.0 error codes (spec §5.1).
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

const JSONRPC_VERSION: &str = "2.0";

/// A method-not-found or malformed-request error, ready to serialize.
#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Answers one JSON-RPC method call that isn't handled at the transport
/// level. `params` is `Value::Null` when the request carried none.
pub trait Handler {
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError>;
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcErrorBody {
    code: i32,
    message: String,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcErrorBody>,
}

/// Dispatch one already-parsed JSON-RPC message to `handler`. `None` for a
/// notification (no `id`) — the caller must not write a response body for
/// those. A message that doesn't even look like a JSON-RPC request (no
/// `method`) still gets an `INVALID_REQUEST` response, with `id` preserved
/// when present.
pub fn dispatch(handler: &mut impl Handler, message: Value) -> Option<Value> {
    // Only the id is needed on the error path; don't clone the whole body.
    let id_on_error = message.get("id").cloned().unwrap_or(Value::Null);
    let request: RpcRequest = match serde_json::from_value(message) {
        Ok(r) => r,
        Err(_) => {
            return Some(response_value(
                id_on_error,
                Err(RpcError::new(INVALID_REQUEST, "invalid request")),
            ));
        }
    };
    let is_notification = request.id.is_none();
    let result = route(&request.method, request.params, handler);
    if is_notification {
        return None;
    }
    Some(response_value(request.id.unwrap_or(Value::Null), result))
}

/// Methods answered by the transport itself, ahead of the application
/// [`Handler`]: a bare liveness check and the lifecycle notifications every
/// MCP client sends that carry nothing worth acting on.
fn route(method: &str, params: Value, handler: &mut impl Handler) -> Result<Value, RpcError> {
    match method {
        "ping" => Ok(json!({})),
        "notifications/initialized" | "notifications/cancelled" => Ok(Value::Null),
        _ => handler.handle(method, params),
    }
}

fn response_value(id: Value, result: Result<Value, RpcError>) -> Value {
    let response = match result {
        Ok(value) => RpcResponse {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(value),
            error: None,
        },
        Err(e) => RpcResponse {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(RpcErrorBody {
                code: e.code,
                message: e.message,
            }),
        },
    };
    // A result that fails to serialize (shouldn't happen, but a tool result
    // is arbitrary JSON built at runtime) shouldn't take the connection
    // down — fall back to a plain internal-error response instead.
    serde_json::to_value(&response).unwrap_or_else(|_| {
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": Value::Null,
            "error": {"code": INTERNAL_ERROR, "message": "internal error"},
        })
    })
}

#[cfg(test)]
#[path = "jsonrpc_test.rs"]
mod tests;
