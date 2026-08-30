//! Minimal JSON-RPC 2.0 transport: newline-delimited requests in, replies
//! out. Knows only the protocol-level methods (`ping`, the lifecycle
//! notifications MCP clients send); everything else is handed to a
//! [`Handler`].

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Standard JSON-RPC 2.0 error codes (spec §5.1).
pub const PARSE_ERROR: i32 = -32700;
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

/// Read newline-delimited JSON-RPC requests from `reader`, dispatch each to
/// `handler`, and write the replies to `writer`. Runs until `reader` hits
/// EOF. Requests with no `id` are notifications: they're still dispatched
/// (for any side effect) but never get a response line.
pub fn run<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    handler: &mut impl Handler,
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        dispatch_line(trimmed, handler, &mut writer)?;
    }
}

fn dispatch_line(
    line: &str,
    handler: &mut impl Handler,
    writer: &mut impl Write,
) -> io::Result<()> {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return write_response(
                writer,
                Value::Null,
                Err(RpcError::new(PARSE_ERROR, "parse error")),
            );
        }
    };
    let request: RpcRequest = match serde_json::from_value(value.clone()) {
        Ok(r) => r,
        Err(_) => {
            let id = value.get("id").cloned().unwrap_or(Value::Null);
            return write_response(
                writer,
                id,
                Err(RpcError::new(INVALID_REQUEST, "invalid request")),
            );
        }
    };
    let is_notification = request.id.is_none();
    let result = route(&request.method, request.params, handler);
    if is_notification {
        return Ok(());
    }
    write_response(writer, request.id.unwrap_or(Value::Null), result)
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

fn write_response(
    writer: &mut impl Write,
    id: Value,
    result: Result<Value, RpcError>,
) -> io::Result<()> {
    let response = match result {
        Ok(value) => RpcResponse {
            jsonrpc: JSONRPC_VERSION,
            id: id.clone(),
            result: Some(value),
            error: None,
        },
        Err(e) => RpcResponse {
            jsonrpc: JSONRPC_VERSION,
            id: id.clone(),
            result: None,
            error: Some(RpcErrorBody {
                code: e.code,
                message: e.message,
            }),
        },
    };
    // A result that fails to serialize (shouldn't happen, but a tool result
    // is arbitrary JSON built at runtime) shouldn't kill the whole
    // connection — fall back to a plain internal-error response instead.
    let mut bytes = serde_json::to_vec(&response).unwrap_or_else(|_| {
        let fallback = RpcResponse {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(RpcErrorBody {
                code: INTERNAL_ERROR,
                message: "internal error".into(),
            }),
        };
        serde_json::to_vec(&fallback).expect("id and a plain error body always serialize")
    });
    bytes.push(b'\n');
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
#[path = "jsonrpc_test.rs"]
mod tests;
