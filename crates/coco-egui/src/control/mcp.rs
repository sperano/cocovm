//! The MCP application methods: `initialize`, `tools/list`, `tools/call`.
//! Everything else routes to [`crate::control::jsonrpc::dispatch`] first,
//! which answers `ping` and the lifecycle notifications itself.

use serde_json::{Value, json};

use super::jsonrpc::{Handler, METHOD_NOT_FOUND, RpcError};
use super::tools;
use super::tools::Backend;

pub const PROTOCOL_VERSION_2024_11_05: &str = "2024-11-05";
pub const PROTOCOL_VERSION_2025_03_26: &str = "2025-03-26";
pub const PROTOCOL_VERSION_2025_06_18: &str = "2025-06-18";

/// MCP features that differ between the protocol versions this server offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProtocolVersion {
    November2024,
    March2025,
    June2025,
}

impl ProtocolVersion {
    pub(crate) fn negotiate(params: &Value) -> Self {
        match params.get("protocolVersion").and_then(Value::as_str) {
            Some(PROTOCOL_VERSION_2024_11_05) => Self::November2024,
            Some(PROTOCOL_VERSION_2025_03_26) => Self::March2025,
            Some(PROTOCOL_VERSION_2025_06_18) => Self::June2025,
            _ => Self::June2025,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::November2024 => PROTOCOL_VERSION_2024_11_05,
            Self::March2025 => PROTOCOL_VERSION_2025_03_26,
            Self::June2025 => PROTOCOL_VERSION_2025_06_18,
        }
    }

    pub(crate) fn accepts_batches(self) -> bool {
        self == Self::March2025
    }

    fn supports_tool_annotations(self) -> bool {
        self != Self::November2024
    }
}

const INSTRUCTIONS: &str = "cocovm's built-in MCP server drives the VMs the app manages directly. \
Call list_vms first to find a VM's slug (most tools take an optional `vm` argument that can be \
omitted when only one VM is running). The VM's text screen is 32x16 characters by default, or \
40/80 columns in CoCo 3 hi-res text modes. type_text ends a line with \"\\n\" to press ENTER. \
After typing or pressing keys, wait a few video fields (see the `wait` tool) before reading the \
screen, since the ROM's keyboard scan and screen redraw both take real emulated time.";

/// Handles the MCP-specific methods; everything else is [`METHOD_NOT_FOUND`].
pub struct Mcp {
    backend: Box<dyn Backend>,
    protocol_version: ProtocolVersion,
}

impl Mcp {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            protocol_version: ProtocolVersion::June2025,
        }
    }

    pub(crate) fn set_protocol_version(&mut self, protocol_version: ProtocolVersion) {
        self.protocol_version = protocol_version;
    }
}

impl Handler for Mcp {
    fn handle(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => {
                self.protocol_version = ProtocolVersion::negotiate(&params);
                Ok(initialize_result(self.protocol_version))
            }
            "tools/list" => Ok(tools::list(
                self.protocol_version.supports_tool_annotations(),
            )),
            "tools/call" => tools::call(self.backend.as_mut(), params),
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            )),
        }
    }
}

fn initialize_result(protocol_version: ProtocolVersion) -> Value {
    json!({
        "protocolVersion": protocol_version.as_str(),
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "cocovm", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

#[cfg(test)]
#[path = "mcp_test.rs"]
mod tests;
