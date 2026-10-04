//! The MCP application methods: `initialize`, `tools/list`, `tools/call`.
//! Everything else routes to [`crate::control::jsonrpc::dispatch`] first,
//! which answers `ping` and the lifecycle notifications itself.

use serde_json::{Value, json};

use super::jsonrpc::{Handler, METHOD_NOT_FOUND, RpcError};
use super::tools;
use super::tools::Backend;

/// Protocol version this server speaks when a client doesn't ask for one of
/// the versions it understands.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
/// First protocol version with tool `outputSchema`/`structuredContent`.
/// Versions are ISO dates, so string order is release order.
const STRUCTURED_OUTPUT_SINCE: &str = "2025-06-18";

const INSTRUCTIONS: &str = "cocovm's built-in MCP server drives the VMs the app manages directly. \
Call list_vms first to find a VM's slug (most tools take an optional `vm` argument that can be \
omitted when only one VM is running). The VM's text screen is 32x16 characters by default, or \
40/80 columns in CoCo 3 hi-res text modes. type_text ends a line with \"\\n\" to press ENTER. \
After typing or pressing keys, wait a few video fields (see the `wait` tool) before reading the \
screen, since the ROM's keyboard scan and screen redraw both take real emulated time.";

/// `version` as this server's own `&'static str`, or `None` when it isn't
/// one of [`SUPPORTED_PROTOCOL_VERSIONS`].
pub fn supported_version(version: &str) -> Option<&'static str> {
    SUPPORTED_PROTOCOL_VERSIONS
        .into_iter()
        .find(|&known| known == version)
}

/// The version `initialize` answers with: the client's `requested` one when
/// supported, else [`PROTOCOL_VERSION`].
pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    requested
        .and_then(supported_version)
        .unwrap_or(PROTOCOL_VERSION)
}

/// Handles the MCP-specific methods; everything else is [`METHOD_NOT_FOUND`].
pub struct Mcp {
    backend: Box<dyn Backend>,
    /// Version in effect for the request being handled; the transport sets
    /// it per request, since sessions outlive connections.
    protocol_version: &'static str,
}

impl Mcp {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            protocol_version: PROTOCOL_VERSION,
        }
    }

    /// Set the protocol version the next requests are answered in.
    pub fn set_protocol_version(&mut self, version: &'static str) {
        self.protocol_version = version;
    }

    /// Whether tool definitions carry `outputSchema` and results carry
    /// `structuredContent`; older clients get the text content alone.
    fn structured_output(&self) -> bool {
        self.protocol_version >= STRUCTURED_OUTPUT_SINCE
    }
}

impl Handler for Mcp {
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(initialize_result(&params)),
            "tools/list" => Ok(tools::list(self.structured_output())),
            "tools/call" => {
                let structured_output = self.structured_output();
                tools::call(self.backend.as_mut(), params, structured_output)
            }
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            )),
        }
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let protocol_version = negotiate_version(requested);
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "cocovm", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

#[cfg(test)]
#[path = "mcp_test.rs"]
mod tests;
