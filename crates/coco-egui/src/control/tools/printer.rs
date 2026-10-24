use serde::Deserialize;
use serde_json::{Value, json};

use super::{Backend, finish, parse_args, structured_result};
use crate::control::protocol::{Action, Reply, Request};

#[derive(Deserialize)]
struct ReadPrinterArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    clear: bool,
}

pub(super) fn dispatch(backend: &mut dyn Backend, args: Value) -> Result<Value, super::RpcError> {
    let ReadPrinterArgs { vm, clear } = parse_args(args)?;
    let request = Request {
        vm,
        action: Action::ReadPrinter { clear },
    };
    Ok(finish(backend, request, |reply| match reply {
        Reply::Printer(output) => {
            let text = if output.truncated {
                format!("[Earlier printer output was truncated.]\n{}", output.text)
            } else {
                output.text.clone()
            };
            Some(structured_result(
                text,
                json!({"text": output.text, "truncated": output.truncated}),
            ))
        }
        _ => None,
    }))
}

#[cfg(test)]
#[path = "printer_test.rs"]
mod tests;
