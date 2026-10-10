//! The `load_binary` tool: source decoding and complete DECB validation.

use std::fs;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use coco_core::decb::{DecbBinary, DecbSegment};
use serde::Deserialize;
use serde_json::Value;

use super::{Backend, done, error_result, finish, parse_args};
use crate::control::jsonrpc::{INVALID_PARAMS, RpcError};
use crate::control::protocol::{Action, Request};

#[derive(Deserialize)]
struct LoadBinaryArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    bytes: Option<String>,
    #[serde(default)]
    address: Option<u16>,
    #[serde(default)]
    exec: bool,
}

pub(super) fn dispatch(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let args: LoadBinaryArgs = parse_args(args)?;
    let source = match read_source(args.path, args.bytes) {
        Ok(source) => source,
        Err(SourceError::Invalid(message)) => {
            return Err(RpcError::new(INVALID_PARAMS, message));
        }
        Err(SourceError::Read(message)) => return Ok(error_result(message)),
    };

    let (segments, exec_address, message) = match args.address {
        Some(address) => raw_load(source, address, args.exec),
        None => match decb_load(source, args.exec) {
            Ok(load) => load,
            Err(message) => return Ok(error_result(message)),
        },
    };
    let request = Request {
        vm: args.vm,
        action: Action::LoadBinary {
            segments,
            exec_address,
        },
    };
    Ok(finish(backend, request, |reply| done(reply, &message)))
}

enum SourceError {
    Invalid(String),
    Read(String),
}

fn read_source(path: Option<String>, bytes: Option<String>) -> Result<Vec<u8>, SourceError> {
    match (path, bytes) {
        (Some(path), None) => fs::read(&path).map_err(|error| {
            SourceError::Read(format!("could not read binary at {path:?}: {error}"))
        }),
        (None, Some(encoded)) => BASE64
            .decode(encoded)
            .map_err(|error| SourceError::Read(format!("invalid base64 in `bytes`: {error}"))),
        _ => Err(SourceError::Invalid(
            "invalid arguments: provide exactly one of `path` or `bytes`".to_string(),
        )),
    }
}

fn raw_load(
    bytes: Vec<u8>,
    address: u16,
    execute: bool,
) -> (Vec<DecbSegment>, Option<u16>, String) {
    let length = bytes.len();
    let exec_address = execute.then_some(address);
    let message = format_message(
        format!("Loaded {length} raw byte(s) at ${address:04X}."),
        exec_address,
    );
    (vec![DecbSegment { address, bytes }], exec_address, message)
}

fn decb_load(
    bytes: Vec<u8>,
    execute: bool,
) -> Result<(Vec<DecbSegment>, Option<u16>, String), String> {
    let binary =
        DecbBinary::parse(&bytes).map_err(|error| format!("invalid DECB binary: {error}"))?;
    let byte_count: usize = binary
        .segments
        .iter()
        .map(|segment| segment.bytes.len())
        .sum();
    let segment_count = binary.segments.len();
    let exec_address = execute.then_some(binary.exec_address);
    let message = format_message(
        format!(
            "Loaded {byte_count} byte(s) from {segment_count} DECB segment(s); \
             execution address ${:04X}.",
            binary.exec_address
        ),
        exec_address,
    );
    Ok((binary.segments, exec_address, message))
}

fn format_message(loaded: String, exec_address: Option<u16>) -> String {
    match exec_address {
        Some(address) => format!("{loaded} Started execution at ${address:04X}."),
        None => format!("{loaded} The program counter was unchanged."),
    }
}

#[cfg(test)]
#[path = "load_binary_test.rs"]
mod tests;
