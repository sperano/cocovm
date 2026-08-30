//! Maps `tools/list` and `tools/call` onto the control protocol.

use coco_control::{Action, ControlError, Reply, Request, Stick, VmInfo};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::backend::{Backend, unreachable_message};
use crate::jsonrpc::{INVALID_PARAMS, RpcError};
use crate::tool_defs;

/// Bytes shown per line of a [`peek`] hex dump.
const HEX_DUMP_WIDTH: usize = 16;

pub fn list() -> Value {
    json!({"tools": tool_defs::definitions()})
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// Dispatch one `tools/call`. `Err` only for malformed params (unknown tool,
/// wrong argument shape); a tool or backend failure is `Ok` with `isError`.
pub fn call(backend: &mut dyn Backend, port: u16, params: Value) -> Result<Value, RpcError> {
    let call_params: CallParams = serde_json::from_value(params)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid tools/call params: {e}")))?;
    let args = match call_params.arguments {
        Value::Null => json!({}),
        other => other,
    };
    match call_params.name.as_str() {
        "list_vms" => dispatch_list_vms(backend, port, args),
        "start_vm" => dispatch_start_vm(backend, port, args),
        "screen_text" => dispatch_screen_text(backend, port, args),
        "screenshot" => dispatch_screenshot(backend, port, args),
        "type_text" => dispatch_type_text(backend, port, args),
        "press_keys" => dispatch_press_keys(backend, port, args),
        "joystick" => dispatch_joystick(backend, port, args),
        "insert_disk" => dispatch_insert_disk(backend, port, args),
        "eject_disk" => dispatch_eject_disk(backend, port, args),
        "reset" => dispatch_reset(backend, port, args),
        "set_running" => dispatch_set_running(backend, port, args),
        "wait" => dispatch_wait(backend, port, args),
        "peek" => dispatch_peek(backend, port, args),
        "poke" => dispatch_poke(backend, port, args),
        other => Err(RpcError::new(
            INVALID_PARAMS,
            format!("unknown tool: {other}"),
        )),
    }
}

fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, RpcError> {
    serde_json::from_value(args)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid arguments: {e}")))
}

/// Runs `req` against `backend` and turns the result into a tool-result
/// value: `on_ok` reads the expected [`Reply`] variant, `Err` becomes an
/// `isError` text block, and an unexpected `Reply` variant does too.
fn finish(
    backend: &mut dyn Backend,
    port: u16,
    req: Request,
    on_ok: impl FnOnce(Reply) -> Option<Value>,
) -> Value {
    match backend.call(&req) {
        Ok(reply) => on_ok(reply)
            .unwrap_or_else(|| error_result("cocovm returned an unexpected reply".into())),
        Err(e) => error_result(error_text(e, port)),
    }
}

fn text_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn error_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn done(reply: Reply, message: &str) -> Option<Value> {
    matches!(reply, Reply::Done).then(|| text_result(message.to_string()))
}

fn error_text(err: ControlError, port: u16) -> String {
    match err {
        ControlError::Io(_) | ControlError::Disconnected => unreachable_message(port),
        ControlError::Remote(msg) => msg,
        ControlError::Timeout => err.to_string(),
    }
}

fn format_vms(vms: &[VmInfo]) -> String {
    vms.iter()
        .map(|v| format!("{} — {} ({})", v.slug, v.name, v.status.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_screen(lines: &[String], mode: &str) -> String {
    format!("```\n{}\n```\n{mode}", lines.join("\n"))
}

fn hex_dump(addr: u16, bytes: &[u8]) -> String {
    bytes
        .chunks(HEX_DUMP_WIDTH)
        .enumerate()
        .map(|(i, chunk)| {
            let line_addr = addr.wrapping_add((i * HEX_DUMP_WIDTH) as u16);
            let hex = chunk
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{line_addr:04X}: {hex}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize)]
struct VmOnly {
    #[serde(default)]
    vm: Option<String>,
}

fn dispatch_list_vms(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let _: VmOnly = parse_args(args)?;
    let req = Request {
        vm: None,
        action: Action::ListVms,
    };
    Ok(finish(backend, port, req, |reply| match reply {
        Reply::Vms(vms) => Some(text_result(format_vms(&vms))),
        _ => None,
    }))
}

#[derive(Deserialize)]
struct StartVmArgs {
    vm: String,
}

fn dispatch_start_vm(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let StartVmArgs { vm } = parse_args(args)?;
    let req = Request {
        vm: Some(vm),
        action: Action::StartVm,
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Started.")))
}

fn dispatch_screen_text(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::ScreenText,
    };
    Ok(finish(backend, port, req, |reply| match reply {
        Reply::Screen { lines, mode } => Some(text_result(format_screen(&lines, &mode))),
        _ => None,
    }))
}

fn dispatch_screenshot(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Screenshot,
    };
    Ok(finish(backend, port, req, |reply| match reply {
        Reply::Screenshot {
            png_base64,
            width,
            height,
        } => Some(json!({
            "content": [
                {"type": "image", "data": png_base64, "mimeType": "image/png"},
                {"type": "text", "text": format!("{width}×{height} framebuffer")}
            ],
            "isError": false
        })),
        _ => None,
    }))
}

#[derive(Deserialize)]
struct TypeTextArgs {
    #[serde(default)]
    vm: Option<String>,
    text: String,
}

fn dispatch_type_text(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let TypeTextArgs { vm, text } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::TypeText { text },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Typed.")))
}

#[derive(Deserialize)]
struct PressKeysArgs {
    #[serde(default)]
    vm: Option<String>,
    keys: Vec<String>,
    #[serde(default)]
    hold_fields: Option<u32>,
}

fn dispatch_press_keys(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let PressKeysArgs {
        vm,
        keys,
        hold_fields,
    } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::PressKeys { keys, hold_fields },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Pressed.")))
}

#[derive(Deserialize)]
struct JoystickArgs {
    #[serde(default)]
    vm: Option<String>,
    stick: Stick,
    #[serde(default)]
    x: Option<u8>,
    #[serde(default)]
    y: Option<u8>,
    #[serde(default)]
    button1: Option<bool>,
    #[serde(default)]
    button2: Option<bool>,
    #[serde(default)]
    release: bool,
}

fn dispatch_joystick(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let JoystickArgs {
        vm,
        stick,
        x,
        y,
        button1,
        button2,
        release,
    } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Joystick {
            stick,
            x,
            y,
            button1,
            button2,
            release,
        },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Set.")))
}

#[derive(Deserialize)]
struct InsertDiskArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
    path: String,
}

fn dispatch_insert_disk(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let InsertDiskArgs { vm, drive, path } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::InsertDisk { drive, path },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Inserted.")))
}

#[derive(Deserialize)]
struct EjectDiskArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
}

fn dispatch_eject_disk(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let EjectDiskArgs { vm, drive } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::EjectDisk { drive },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Ejected.")))
}

#[derive(Deserialize)]
struct ResetArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    hard: bool,
}

fn dispatch_reset(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let ResetArgs { vm, hard } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Reset { hard },
    };
    Ok(finish(backend, port, req, |reply| done(reply, "Reset.")))
}

#[derive(Deserialize)]
struct SetRunningArgs {
    #[serde(default)]
    vm: Option<String>,
    running: bool,
}

fn dispatch_set_running(
    backend: &mut dyn Backend,
    port: u16,
    args: Value,
) -> Result<Value, RpcError> {
    let SetRunningArgs { vm, running } = parse_args(args)?;
    let message = if running { "Resumed." } else { "Paused." };
    let req = Request {
        vm,
        action: Action::SetRunning { running },
    };
    Ok(finish(backend, port, req, |reply| done(reply, message)))
}

#[derive(Deserialize)]
struct WaitArgs {
    #[serde(default)]
    vm: Option<String>,
    fields: u32,
}

fn dispatch_wait(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let WaitArgs { vm, fields } = parse_args(args)?;
    // The app clamps the same way; report what actually elapsed.
    let message = format!(
        "Waited {} fields.",
        fields.min(coco_control::MAX_WAIT_FIELDS)
    );
    let req = Request {
        vm,
        action: Action::Wait { fields },
    };
    Ok(finish(backend, port, req, |reply| done(reply, &message)))
}

#[derive(Deserialize)]
struct PeekArgs {
    #[serde(default)]
    vm: Option<String>,
    addr: u16,
    len: u16,
}

fn dispatch_peek(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let PeekArgs { vm, addr, len } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Peek { addr, len },
    };
    Ok(finish(backend, port, req, move |reply| match reply {
        Reply::Bytes(bytes) => Some(text_result(hex_dump(addr, &bytes))),
        _ => None,
    }))
}

#[derive(Deserialize)]
struct PokeArgs {
    #[serde(default)]
    vm: Option<String>,
    addr: u16,
    bytes: Vec<u8>,
}

fn dispatch_poke(backend: &mut dyn Backend, port: u16, args: Value) -> Result<Value, RpcError> {
    let PokeArgs { vm, addr, bytes } = parse_args(args)?;
    let message = format!("Wrote {} byte(s) at ${addr:04X}.", bytes.len());
    let req = Request {
        vm,
        action: Action::Poke { addr, bytes },
    };
    Ok(finish(backend, port, req, |reply| done(reply, &message)))
}

#[cfg(test)]
#[path = "tools_test.rs"]
mod tests;
