//! The `save_state` and `load_state` tools.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::{Backend, done, finish, parse_args};
use crate::control::jsonrpc::{INVALID_PARAMS, RpcError};
use crate::control::protocol::{Action, Request, StateTarget};

const FIRST_QUICK_SLOT: usize = 1;

#[derive(Deserialize)]
struct StateArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default)]
    slot: Option<usize>,
}

pub(super) fn dispatch_save(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    dispatch(
        backend,
        args,
        |target| Action::SaveState { target },
        "Saved state.",
    )
}

pub(super) fn dispatch_load(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    dispatch(
        backend,
        args,
        |target| Action::LoadState { target },
        "Loaded state.",
    )
}

fn dispatch(
    backend: &mut dyn Backend,
    args: Value,
    action: impl FnOnce(StateTarget) -> Action,
    message: &str,
) -> Result<Value, RpcError> {
    let StateArgs { vm, path, slot } = parse_args(args)?;
    let target = parse_target(path, slot)?;
    let request = Request {
        vm,
        action: action(target),
    };
    Ok(finish(backend, request, |reply| done(reply, message)))
}

fn parse_target(path: Option<PathBuf>, slot: Option<usize>) -> Result<StateTarget, RpcError> {
    match (path, slot) {
        (Some(path), None) => Ok(StateTarget::Path(path)),
        (None, Some(slot))
            if (FIRST_QUICK_SLOT..=crate::save_state::QUICK_SLOTS).contains(&slot) =>
        {
            Ok(StateTarget::Slot(slot - FIRST_QUICK_SLOT))
        }
        (None, Some(slot)) => Err(invalid_target(format!(
            "slot must be {FIRST_QUICK_SLOT}..={}, got {slot}",
            crate::save_state::QUICK_SLOTS
        ))),
        _ => Err(invalid_target(
            "provide exactly one of `path` or `slot`".to_string(),
        )),
    }
}

fn invalid_target(message: String) -> RpcError {
    RpcError::new(INVALID_PARAMS, format!("invalid arguments: {message}"))
}

#[cfg(test)]
#[path = "state_test.rs"]
mod tests;
