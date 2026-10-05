//! The `peek` and `poke` tools: logical or physical addressing, and the
//! hex-dump text `peek` returns.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Backend, done, finish, parse_args, structured_result};
use crate::control::jsonrpc::{INVALID_PARAMS, RpcError};
use crate::control::protocol::{Action, MemAddr, Reply, Request};
use crate::debugger::ascii_char;

/// Bytes shown per line of a `peek` hex dump.
const HEX_DUMP_WIDTH: usize = 16;
/// Characters one byte takes in a hex dump line: two digits and a space.
const HEX_CELL_CHARS: usize = 3;
/// Width of a full line's hex column, so a short last line's ASCII column
/// lines up with the ones above it.
const HEX_COLUMN_CHARS: usize = HEX_DUMP_WIDTH * HEX_CELL_CHARS - 1;

/// `peek`'s text: per line, the address, up to [`HEX_DUMP_WIDTH`] bytes in
/// hex, then the same bytes as printable ASCII between bars. Logical
/// addresses wrap past $FFFF; physical ones print six digits.
fn hex_dump(addr: MemAddr, bytes: &[u8]) -> String {
    bytes
        .chunks(HEX_DUMP_WIDTH)
        .enumerate()
        .map(|(i, chunk)| {
            let offset = i * HEX_DUMP_WIDTH;
            let line_addr = match addr {
                MemAddr::Logical(addr) => format!("{:04X}", addr.wrapping_add(offset as u16)),
                MemAddr::Physical(addr) => format!("{:06X}", addr as usize + offset),
            };
            let hex = chunk
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = chunk.iter().copied().map(ascii_char).collect();
            format!("{line_addr}: {hex:<HEX_COLUMN_CHARS$}  |{ascii}|")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool call's `addr`/`physical` pair as a [`MemAddr`]. A logical address
/// above $FFFF is malformed; a physical one is range-checked by the app,
/// which knows the target's installed RAM.
fn mem_addr(addr: u32, physical: bool) -> Result<MemAddr, RpcError> {
    if physical {
        return Ok(MemAddr::Physical(addr));
    }
    u16::try_from(addr).map(MemAddr::Logical).map_err(|_| {
        RpcError::new(
            INVALID_PARAMS,
            format!(
                "invalid arguments: logical address ${addr:X} is above $FFFF; \
                 set `physical` to address RAM directly"
            ),
        )
    })
}

#[derive(Deserialize)]
struct PeekArgs {
    #[serde(default)]
    vm: Option<String>,
    addr: u32,
    len: u16,
    #[serde(default)]
    physical: bool,
}

pub(super) fn dispatch_peek(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let PeekArgs {
        vm,
        addr,
        len,
        physical,
    } = parse_args(args)?;
    let mem = mem_addr(addr, physical)?;
    let req = Request {
        vm,
        action: Action::Peek { addr: mem, len },
    };
    Ok(finish(backend, req, move |reply| match reply {
        Reply::Bytes(bytes) => Some(structured_result(
            hex_dump(mem, &bytes),
            json!({"addr": addr, "physical": physical, "bytes": bytes}),
        )),
        _ => None,
    }))
}

#[derive(Deserialize)]
struct PokeArgs {
    #[serde(default)]
    vm: Option<String>,
    addr: u32,
    bytes: Vec<u8>,
    #[serde(default)]
    physical: bool,
}

pub(super) fn dispatch_poke(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let PokeArgs {
        vm,
        addr,
        bytes,
        physical,
    } = parse_args(args)?;
    let mem = mem_addr(addr, physical)?;
    let message = match mem {
        MemAddr::Logical(addr) => format!("Wrote {} byte(s) at ${addr:04X}.", bytes.len()),
        MemAddr::Physical(addr) => {
            format!("Wrote {} byte(s) at physical ${addr:06X}.", bytes.len())
        }
    };
    let req = Request {
        vm,
        action: Action::Poke { addr: mem, bytes },
    };
    Ok(finish(backend, req, |reply| done(reply, &message)))
}

#[cfg(test)]
#[path = "memory_test.rs"]
mod tests;
