use serde_json::{Value, json};

use super::*;
use crate::control::Reply;
use crate::control::tools::{MockBackend, call};

const STRUCTURED: bool = true;

fn call_tool(mock: &mut MockBackend, name: &str, arguments: Value) -> Result<Value, RpcError> {
    call(
        mock,
        json!({"name": name, "arguments": arguments}),
        STRUCTURED,
    )
}

#[test]
fn path_targets_are_forwarded_with_the_optional_vm() {
    for name in ["save_state", "load_state"] {
        let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
        let result = call_tool(
            &mut mock,
            name,
            json!({"vm": "demo", "path": "/tmp/demo.ccstate"}),
        )
        .unwrap();

        assert_eq!(result["isError"], json!(false));
        assert_eq!(mock.calls[0].vm.as_deref(), Some("demo"));
        let target = match &mock.calls[0].action {
            Action::SaveState { target } | Action::LoadState { target } => target,
            other => panic!("unexpected action: {other:?}"),
        };
        assert_eq!(target, &StateTarget::Path("/tmp/demo.ccstate".into()));
    }
}

#[test]
fn external_slots_are_one_based_and_become_internal_indexes() {
    for (external, internal) in [
        (FIRST_QUICK_SLOT, 0),
        (
            crate::save_state::QUICK_SLOTS,
            crate::save_state::QUICK_SLOTS - FIRST_QUICK_SLOT,
        ),
    ] {
        let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
        call_tool(&mut mock, "save_state", json!({"slot": external})).unwrap();

        assert_eq!(
            mock.calls[0].action,
            Action::SaveState {
                target: StateTarget::Slot(internal)
            }
        );
    }
}

#[test]
fn exactly_one_valid_target_is_required() {
    let invalid = [
        json!({}),
        json!({"path": "x.ccstate", "slot": 1}),
        json!({"slot": 0}),
        json!({"slot": crate::save_state::QUICK_SLOTS + 1}),
    ];
    for arguments in invalid {
        let mut mock = MockBackend::new(vec![]);
        let error = call_tool(&mut mock, "load_state", arguments).unwrap_err();
        assert_eq!(error.code, INVALID_PARAMS);
        assert!(mock.calls.is_empty());
    }
}

#[test]
fn backend_failures_are_tool_errors() {
    let mut mock = MockBackend::new(vec![Err("dirty disk write-back failed".into())]);
    let result = call_tool(&mut mock, "save_state", json!({"slot": 1})).unwrap();

    assert_eq!(result["isError"], json!(true));
    assert_eq!(
        result["content"][0]["text"],
        json!("dirty disk write-back failed")
    );
}
