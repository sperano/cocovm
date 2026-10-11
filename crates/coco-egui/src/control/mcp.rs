//! The MCP application methods: `initialize`, `tools/list`, `tools/call`,
//! and the `resources/*` reads. Everything else routes to
//! [`crate::control::jsonrpc::dispatch`] first, which answers `ping` and the
//! lifecycle notifications itself.

use serde_json::{Value, json};

use super::jsonrpc::{Handler, METHOD_NOT_FOUND, RpcError};
use super::tools::Backend;
use super::{resources, tools};

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
        params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .and_then(Self::parse)
            .unwrap_or(Self::June2025)
    }

    /// `version` when it names one of the versions this server speaks, as
    /// in an `MCP-Protocol-Version` header; `None` otherwise.
    pub(crate) fn parse(version: &str) -> Option<Self> {
        match version {
            PROTOCOL_VERSION_2024_11_05 => Some(Self::November2024),
            PROTOCOL_VERSION_2025_03_26 => Some(Self::March2025),
            PROTOCOL_VERSION_2025_06_18 => Some(Self::June2025),
            _ => None,
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

    /// Whether tool definitions carry `outputSchema` and results carry
    /// `structuredContent`; older clients get the text content alone.
    fn supports_structured_output(self) -> bool {
        self == Self::June2025
    }

    /// Whether resources carry a display `title` next to their `name`.
    fn supports_resource_titles(self) -> bool {
        self == Self::June2025
    }
}

const INSTRUCTIONS: &str = "cocovm's built-in MCP server drives the VMs the app manages directly. \
Call list_vms first to find a VM's slug (most tools take an optional `vm` argument that can be \
omitted when only one VM is running). The VM's text screen is 32x16 characters by default, or \
40/80 columns in CoCo 3 hi-res text modes. type_text ends a line with \"\\n\" to press ENTER. \
After typing or pressing keys, wait a few video fields (see the `wait` tool) before reading the \
screen, since the ROM's keyboard scan and screen redraw both take real emulated time. The same \
screen is also readable as resources: cocovm://vm/<slug>/screen.txt (text) and \
cocovm://vm/<slug>/screen.png (PNG), one pair per VM in resources/list.";

/// Handles the MCP-specific methods; everything else is [`METHOD_NOT_FOUND`].
pub struct Mcp {
    backend: Box<dyn Backend>,
    /// Version in effect for the request being handled; the transport sets
    /// it per request, since sessions outlive connections.
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
                self.protocol_version.supports_structured_output(),
            )),
            "tools/call" => tools::call(
                self.backend.as_mut(),
                params,
                self.protocol_version.supports_structured_output(),
            ),
            "resources/list" => resources::list(
                self.backend.as_mut(),
                params,
                self.protocol_version.supports_resource_titles(),
            ),
            "resources/templates/list" => Ok(resources::templates(
                self.protocol_version.supports_resource_titles(),
            )),
            "resources/read" => resources::read(self.backend.as_mut(), params),
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
        "capabilities": {"tools": {}, "resources": resources::capability()},
        "serverInfo": {"name": "cocovm", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

#[cfg(test)]
#[path = "mcp_test.rs"]
mod tests;
