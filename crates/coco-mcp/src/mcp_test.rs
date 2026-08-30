use super::*;
use crate::backend::MockBackend;
use crate::jsonrpc::{self, METHOD_NOT_FOUND};
use crate::tool_defs;
use coco_control::Reply;

const PORT: u16 = 6809;

fn mcp_with(responses: Vec<Result<Reply, coco_control::ControlError>>) -> Mcp {
    Mcp::new(Box::new(MockBackend::new(responses)), PORT)
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
    assert_eq!(result["serverInfo"]["name"], json!("cocovm-mcp"));
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
fn unrecognized_method_is_method_not_found() {
    let mut mcp = mcp_with(vec![]);
    let err = mcp.handle("resources/list", Value::Null).unwrap_err();
    assert_eq!(err.code, METHOD_NOT_FOUND);
}

#[test]
fn full_session_over_the_jsonrpc_loop() {
    let mut mcp = mcp_with(vec![Ok(Reply::Vms(vec![]))]);
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"list_vms\",\"arguments\":{}}}\n",
    );
    let mut output = Vec::new();
    jsonrpc::run(input.as_bytes(), &mut output, &mut mcp).unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Three replies: the notification produced none.
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["id"], json!(1));
    assert_eq!(lines[0]["result"]["protocolVersion"], json!("2025-06-18"));
    assert_eq!(lines[1]["id"], json!(2));
    assert_eq!(
        lines[1]["result"]["tools"].as_array().unwrap().len(),
        tool_defs::definitions().len()
    );
    assert_eq!(lines[2]["id"], json!(3));
    assert_eq!(lines[2]["result"]["isError"], json!(false));
}
