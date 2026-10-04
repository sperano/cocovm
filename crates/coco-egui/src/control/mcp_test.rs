use super::*;
use crate::control::jsonrpc::{self, METHOD_NOT_FOUND};
use crate::control::protocol::Reply;
use crate::control::tool_defs;
use crate::control::tools::MockBackend;

fn mcp_with(responses: Vec<Result<Reply, String>>) -> Mcp {
    Mcp::new(Box::new(MockBackend::new(responses)))
}

#[test]
fn initialize_echoes_a_supported_protocol_version() {
    let mut mcp = mcp_with(vec![]);
    let result = mcp
        .handle(
            "initialize",
            json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {}}),
        )
        .unwrap();
    assert_eq!(result["protocolVersion"], json!("2024-11-05"));
    assert_eq!(result["serverInfo"]["name"], json!("cocovm"));
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("list_vms")
    );
}

#[test]
fn initialize_falls_back_for_an_unrecognized_protocol_version() {
    let mut mcp = mcp_with(vec![]);
    let result = mcp
        .handle("initialize", json!({"protocolVersion": "1999-01-01"}))
        .unwrap();
    assert_eq!(result["protocolVersion"], json!(PROTOCOL_VERSION));
}

#[test]
fn tools_list_reports_every_tool() {
    let mut mcp = mcp_with(vec![]);
    let result = mcp.handle("tools/list", Value::Null).unwrap();
    assert_eq!(
        result["tools"].as_array().unwrap().len(),
        tool_defs::definitions().len()
    );
}

#[test]
fn tools_call_dispatches_through_the_backend() {
    let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
    let result = mcp
        .handle("tools/call", json!({"name": "list_vms", "arguments": {}}))
        .unwrap();
    assert_eq!(result["isError"], json!(false));
}

#[test]
fn negotiate_version_accepts_only_supported_versions() {
    assert_eq!(negotiate_version(Some("2025-03-26")), "2025-03-26");
    assert_eq!(negotiate_version(Some("2099-01-01")), PROTOCOL_VERSION);
    assert_eq!(negotiate_version(None), PROTOCOL_VERSION);
    assert_eq!(supported_version("2024-11-05"), Some("2024-11-05"));
    assert_eq!(supported_version("2099-01-01"), None);
}

fn declares_output_schema(mcp: &mut Mcp) -> bool {
    let result = mcp.handle("tools/list", Value::Null).unwrap();
    result["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t.get("outputSchema").is_some())
}

fn list_vms_is_structured(mcp: &mut Mcp) -> bool {
    let result = mcp
        .handle("tools/call", json!({"name": "list_vms", "arguments": {}}))
        .unwrap();
    result.get("structuredContent").is_some()
}

#[test]
fn structured_output_starts_at_protocol_2025_06_18() {
    for (version, structured) in [
        ("2024-11-05", false),
        ("2025-03-26", false),
        ("2025-06-18", true),
    ] {
        let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
        mcp.set_protocol_version(version);
        assert_eq!(declares_output_schema(&mut mcp), structured, "{version}");
        assert_eq!(list_vms_is_structured(&mut mcp), structured, "{version}");
    }
}

#[test]
fn unrecognized_method_is_method_not_found() {
    let mut mcp = mcp_with(vec![]);
    let err = mcp.handle("resources/list", Value::Null).unwrap_err();
    assert_eq!(err.code, METHOD_NOT_FOUND);
}

#[test]
fn full_session_over_the_jsonrpc_dispatcher() {
    let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
    let messages = vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_vms", "arguments": {}}}),
    ];
    let responses: Vec<Value> = messages
        .into_iter()
        .filter_map(|m| jsonrpc::dispatch(&mut mcp, m))
        .collect();
    // Four messages in, three replies out: the notification produced none.
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        json!("2025-06-18")
    );
    assert_eq!(responses[1]["id"], json!(2));
    assert_eq!(
        responses[1]["result"]["tools"].as_array().unwrap().len(),
        tool_defs::definitions().len()
    );
    assert_eq!(responses[2]["id"], json!(3));
    assert_eq!(responses[2]["result"]["isError"], json!(false));
}
