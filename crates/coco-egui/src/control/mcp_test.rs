use super::*;
use crate::control::jsonrpc::{self, METHOD_NOT_FOUND};
use crate::control::protocol::{ControlError, Reply};
use crate::control::tool_defs;
use crate::control::tools::MockBackend;

fn mcp_with(responses: Vec<Result<Reply, ControlError>>) -> Mcp {
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
    assert_eq!(
        result["protocolVersion"],
        json!(PROTOCOL_VERSION_2025_06_18)
    );
}

#[test]
fn tools_list_reports_every_tool() {
    let mut mcp = mcp_with(vec![]);
    let result = mcp.handle("tools/list", Value::Null).unwrap();
    assert_eq!(
        result["tools"].as_array().unwrap().len(),
        tool_defs::definitions(true).len()
    );
}

#[test]
fn tools_list_omits_annotations_for_2024_clients() {
    let mut mcp = mcp_with(vec![]);
    mcp.handle(
        "initialize",
        json!({"protocolVersion": PROTOCOL_VERSION_2024_11_05}),
    )
    .unwrap();

    let result = mcp.handle("tools/list", Value::Null).unwrap();

    assert!(
        result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool.get("annotations").is_none())
    );
}

#[test]
fn tools_list_includes_annotations_for_2025_clients() {
    for protocol_version in [PROTOCOL_VERSION_2025_03_26, PROTOCOL_VERSION_2025_06_18] {
        let mut mcp = mcp_with(vec![]);
        mcp.handle("initialize", json!({"protocolVersion": protocol_version}))
            .unwrap();
        let result = mcp.handle("tools/list", Value::Null).unwrap();

        assert!(
            result["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["annotations"].is_object())
        );
    }
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
fn protocol_version_parses_only_supported_versions() {
    for version in [
        ProtocolVersion::November2024,
        ProtocolVersion::March2025,
        ProtocolVersion::June2025,
    ] {
        assert_eq!(ProtocolVersion::parse(version.as_str()), Some(version));
    }
    assert_eq!(ProtocolVersion::parse("2099-01-01"), None);
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
        (ProtocolVersion::November2024, false),
        (ProtocolVersion::March2025, false),
        (ProtocolVersion::June2025, true),
    ] {
        let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
        mcp.set_protocol_version(version);
        assert_eq!(declares_output_schema(&mut mcp), structured, "{version:?}");
        assert_eq!(list_vms_is_structured(&mut mcp), structured, "{version:?}");
    }
}

#[test]
fn unrecognized_method_is_method_not_found() {
    let mut mcp = mcp_with(vec![]);
    let err = mcp.handle("prompts/list", Value::Null).unwrap_err();
    assert_eq!(err.code, METHOD_NOT_FOUND);
}

#[test]
fn initialize_offers_resources_without_subscriptions() {
    let mut mcp = mcp_with(vec![]);
    let result = mcp
        .handle("initialize", json!({"protocolVersion": "2025-06-18"}))
        .unwrap();
    assert_eq!(
        result["capabilities"]["resources"],
        json!({"subscribe": false, "listChanged": false})
    );
    assert!(
        result["instructions"]
            .as_str()
            .unwrap()
            .contains("cocovm://vm/<slug>/screen.txt")
    );
}

#[test]
fn resource_methods_dispatch_through_the_backend() {
    let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
    let listed = mcp.handle("resources/list", Value::Null).unwrap();
    assert_eq!(listed["resources"], json!([]));

    let templates = mcp.handle("resources/templates/list", Value::Null).unwrap();
    assert_eq!(templates["resourceTemplates"].as_array().unwrap().len(), 2);

    let err = mcp
        .handle("resources/read", json!({"uri": "file:///etc/passwd"}))
        .unwrap_err();
    assert_eq!(err.code, resources::RESOURCE_NOT_FOUND);
}

/// Subscriptions aren't offered, so the subscribe methods stay unknown.
#[test]
fn resource_subscriptions_are_method_not_found() {
    let mut mcp = mcp_with(vec![]);
    for method in ["resources/subscribe", "resources/unsubscribe"] {
        let err = mcp
            .handle(method, json!({"uri": "cocovm://vm/vm0/screen.txt"}))
            .unwrap_err();
        assert_eq!(err.code, METHOD_NOT_FOUND, "{method}");
    }
}

#[test]
fn resource_titles_start_at_protocol_2025_06_18() {
    for (version, titled) in [
        (ProtocolVersion::November2024, false),
        (ProtocolVersion::March2025, false),
        (ProtocolVersion::June2025, true),
    ] {
        let mut mcp = mcp_with(vec![]);
        mcp.set_protocol_version(version);
        let templates = mcp.handle("resources/templates/list", Value::Null).unwrap();
        let has_title = templates["resourceTemplates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t.get("title").is_some());
        assert_eq!(has_title, titled, "{version:?}");
    }
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
        tool_defs::definitions(true).len()
    );
    assert_eq!(responses[2]["id"], json!(3));
    assert_eq!(responses[2]["result"]["isError"], json!(false));
}
