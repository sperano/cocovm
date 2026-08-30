//! The MCP application methods: `initialize`, `tools/list`, `tools/call`.
//! Everything else routes to [`crate::jsonrpc::run`] first, which answers
//! `ping` and the lifecycle notifications itself.

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::jsonrpc::{Handler, METHOD_NOT_FOUND, RpcError};
use crate::tools;

/// Protocol version this server speaks when a client doesn't ask for one of
/// the versions it understands.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

const INSTRUCTIONS: &str = "cocovm-mcp drives a running cocovm app over its control port; the \
app must already be running before any tool but list_vms is useful. Call list_vms first to find \
a VM's slug (most tools take an optional `vm` argument that can be omitted when only one VM is \
running). The VM's text screen is 32x16 characters by default, or 40/80 columns in CoCo 3 \
hi-res text modes. type_text ends a line with \"\\n\" to press ENTER. After typing or pressing \
keys, wait a few video fields (see the `wait` tool) before reading the screen, since the ROM's \
keyboard scan and screen redraw both take real emulated time.";

/// Handles the MCP-specific methods; everything else is [`METHOD_NOT_FOUND`].
pub struct Mcp {
    backend: Box<dyn Backend>,
    port: u16,
}

impl Mcp {
    pub fn new(backend: Box<dyn Backend>, port: u16) -> Self {
        Self { backend, port }
    }
}

impl Handler for Mcp {
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(initialize_result(&params)),
            "tools/list" => Ok(tools::list()),
            "tools/call" => tools::call(self.backend.as_mut(), self.port, params),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            )),
        }
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let protocol_version = match requested {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        _ => PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "cocovm-mcp", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

#[cfg(test)]
#[path = "mcp_test.rs"]
mod tests;
