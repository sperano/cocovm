//! Maps `tools/list` and `tools/call` onto the [`protocol`] request/reply
//! types.

use coco_core::TextCursor;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::jsonrpc::{INVALID_PARAMS, RpcError};
use super::protocol::{Action, ControlError, Reply, Request, ScreenSnapshot, Stick, TextMatcher};
use super::tool_defs;

mod memory;
mod vms;

/// Tool-definition key for the result schema (protocol 2025-06-18 on).
const OUTPUT_SCHEMA: &str = "outputSchema";
/// Tool-result key for the value matching [`OUTPUT_SCHEMA`].
pub(super) const STRUCTURED_CONTENT: &str = "structuredContent";
/// A tool's error text when the app answers with the wrong [`Reply`] variant.
pub(super) const UNEXPECTED_REPLY: &str = "cocovm returned an unexpected reply";

/// Sends one request and waits for its reply. Exists so `tools::call` can be
/// tested against a mock instead of the real frame-loop queue.
pub trait Backend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError>;
}

/// The `tools/list` result. `include_annotations` adds each tool's
/// behavior hints (protocol 2025-03-26 on); without `structured_output`
/// (clients older than protocol 2025-06-18) the tools carry no
/// `outputSchema`.
pub fn list(include_annotations: bool, structured_output: bool) -> Value {
    let mut tools = tool_defs::definitions(include_annotations);
    if !structured_output {
        for tool in &mut tools {
            remove_key(tool, OUTPUT_SCHEMA);
        }
    }
    json!({"tools": tools})
}

fn remove_key(value: &mut Value, key: &str) {
    if let Some(object) = value.as_object_mut() {
        object.remove(key);
    }
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// Dispatch one `tools/call`. `Err` only for malformed params (unknown tool,
/// wrong argument shape); a tool or backend failure is `Ok` with `isError`.
/// Without `structured_output` the result keeps only its text content.
pub fn call(
    backend: &mut dyn Backend,
    params: Value,
    structured_output: bool,
) -> Result<Value, RpcError> {
    let mut result = dispatch(backend, params)?;
    if !structured_output {
        remove_key(&mut result, STRUCTURED_CONTENT);
    }
    Ok(result)
}

fn dispatch(backend: &mut dyn Backend, params: Value) -> Result<Value, RpcError> {
    let call_params: CallParams = serde_json::from_value(params)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid tools/call params: {e}")))?;
    let args = match call_params.arguments {
        Value::Null => json!({}),
        other => other,
    };
    match call_params.name.as_str() {
        "list_vms" => vms::dispatch_list_vms(backend, args),
        "start_vm" => vms::dispatch_start_vm(backend, args),
        "stop_vm" => vms::dispatch_stop_vm(backend, args),
        "suspend_vm" => vms::dispatch_suspend_vm(backend, args),
        "screen_text" => dispatch_screen_text(backend, args),
        "screenshot" => dispatch_screenshot(backend, args),
        "type_text" => dispatch_type_text(backend, args),
        "press_keys" => dispatch_press_keys(backend, args),
        "joystick" => dispatch_joystick(backend, args),
        "insert_disk" => dispatch_insert_disk(backend, args),
        "eject_disk" => dispatch_eject_disk(backend, args),
        "reset" => dispatch_reset(backend, args),
        "set_running" => dispatch_set_running(backend, args),
        "wait" => dispatch_wait(backend, args),
        "wait_for_text" => dispatch_wait_for_text(backend, args),
        "enter_basic" => super::enter_basic::dispatch(backend, args),
        "peek" => memory::dispatch_peek(backend, args),
        "poke" => memory::dispatch_poke(backend, args),
        other => Err(RpcError::new(
            INVALID_PARAMS,
            format!("unknown tool: {other}"),
        )),
    }
}

pub(super) fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, RpcError> {
    serde_json::from_value(args)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid arguments: {e}")))
}

/// Runs `req` against `backend` and turns the result into a tool-result
/// value: `on_ok` reads the expected [`Reply`] variant, `Err` becomes an
/// `isError` text block, and an unexpected `Reply` variant does too.
fn finish(
    backend: &mut dyn Backend,
    req: Request,
    on_ok: impl FnOnce(Reply) -> Option<Value>,
) -> Value {
    match backend.call(&req) {
        Ok(reply) => on_ok(reply).unwrap_or_else(|| error_result(UNEXPECTED_REPLY.into())),
        Err(error) => error_result(format_control_error(&error)),
    }
}

fn text_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

/// A text result plus the `structuredContent` its tool's `outputSchema`
/// describes. The text stays human-readable rather than the serialized JSON
/// the spec suggests: it is what pre-2025-06-18 clients show the model.
pub(super) fn structured_result(text: String, structured: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": text}],
        STRUCTURED_CONTENT: structured,
        "isError": false
    })
}

pub(super) fn error_result(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

fn done(reply: Reply, message: &str) -> Option<Value> {
    matches!(reply, Reply::Done).then(|| text_result(message.to_string()))
}

pub(super) fn format_screen(screen: &ScreenSnapshot) -> String {
    let cursor = match screen.cursor {
        Some(TextCursor { row, col }) => format!("cursor: row {row}, column {col} (0-based)"),
        None => "cursor: unknown (graphics mode, or BASIC is not driving this screen)".to_string(),
    };
    format!(
        "```\n{}\n```\n{}\n{cursor}",
        screen.lines.join("\n"),
        screen.mode
    )
}

/// `screen_text`'s `structuredContent`; `cursor` is left out when unknown.
pub(super) fn screen_json(snapshot: &ScreenSnapshot) -> Value {
    let mut screen = json!({"lines": snapshot.lines, "mode": snapshot.mode});
    if let Some(TextCursor { row, col }) = snapshot.cursor {
        screen["cursor"] = json!({"row": row, "col": col});
    }
    screen
}

pub(super) fn format_control_error(error: &ControlError) -> String {
    match &error.screen {
        Some(screen) => format!("{}\n{}", error.message, format_screen(screen)),
        None => error.message.clone(),
    }
}

#[derive(Deserialize)]
struct VmOnly {
    #[serde(default)]
    vm: Option<String>,
}

fn dispatch_screen_text(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::ScreenText,
    };
    Ok(finish(backend, req, |reply| match reply {
        Reply::Screen(screen) => Some(structured_result(
            format_screen(&screen),
            screen_json(&screen),
        )),
        _ => None,
    }))
}

fn dispatch_screenshot(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Screenshot,
    };
    Ok(finish(backend, req, |reply| match reply {
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

fn dispatch_type_text(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let TypeTextArgs { vm, text } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::TypeText { text },
    };
    Ok(finish(backend, req, |reply| done(reply, "Typed.")))
}

#[derive(Deserialize)]
struct PressKeysArgs {
    #[serde(default)]
    vm: Option<String>,
    keys: Vec<String>,
    #[serde(default)]
    hold_fields: Option<u32>,
}

fn dispatch_press_keys(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let PressKeysArgs {
        vm,
        keys,
        hold_fields,
    } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::PressKeys { keys, hold_fields },
    };
    Ok(finish(backend, req, |reply| done(reply, "Pressed.")))
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

fn dispatch_joystick(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
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
    Ok(finish(backend, req, |reply| done(reply, "Set.")))
}

#[derive(Deserialize)]
struct InsertDiskArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
    path: String,
}

fn dispatch_insert_disk(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let InsertDiskArgs { vm, drive, path } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::InsertDisk { drive, path },
    };
    Ok(finish(backend, req, |reply| done(reply, "Inserted.")))
}

#[derive(Deserialize)]
struct EjectDiskArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
}

fn dispatch_eject_disk(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let EjectDiskArgs { vm, drive } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::EjectDisk { drive },
    };
    Ok(finish(backend, req, |reply| done(reply, "Ejected.")))
}

#[derive(Deserialize)]
struct ResetArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    hard: bool,
}

fn dispatch_reset(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let ResetArgs { vm, hard } = parse_args(args)?;
    let req = Request {
        vm,
        action: Action::Reset { hard },
    };
    Ok(finish(backend, req, |reply| done(reply, "Reset.")))
}

#[derive(Deserialize)]
struct SetRunningArgs {
    #[serde(default)]
    vm: Option<String>,
    running: bool,
}

fn dispatch_set_running(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let SetRunningArgs { vm, running } = parse_args(args)?;
    let message = if running { "Resumed." } else { "Paused." };
    let req = Request {
        vm,
        action: Action::SetRunning { running },
    };
    Ok(finish(backend, req, |reply| done(reply, message)))
}

#[derive(Deserialize)]
struct WaitArgs {
    #[serde(default)]
    vm: Option<String>,
    fields: u32,
}

fn dispatch_wait(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let WaitArgs { vm, fields } = parse_args(args)?;
    // The app clamps the same way; report what actually elapsed.
    let message = format!("Waited {} fields.", fields.min(super::MAX_WAIT_FIELDS));
    let req = Request {
        vm,
        action: Action::Wait { fields },
    };
    Ok(finish(backend, req, |reply| done(reply, &message)))
}

#[derive(Deserialize)]
struct WaitForTextArgs {
    #[serde(default)]
    vm: Option<String>,
    pattern: String,
    #[serde(default)]
    regex: bool,
    timeout_fields: u32,
}

fn dispatch_wait_for_text(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let WaitForTextArgs {
        vm,
        pattern,
        regex,
        timeout_fields,
    } = parse_args(args)?;
    let matcher = match TextMatcher::new(pattern, regex) {
        Ok(matcher) => matcher,
        Err(message) => return Ok(error_result(message)),
    };
    let req = Request {
        vm,
        action: Action::WaitForText {
            matcher,
            timeout_fields,
        },
    };
    Ok(finish(backend, req, |reply| match reply {
        Reply::Screen(screen) => Some(structured_result(
            format_screen(&screen),
            screen_json(&screen),
        )),
        _ => None,
    }))
}

#[cfg(test)]
pub(crate) struct MockBackend {
    pub(crate) responses: std::collections::VecDeque<Result<Reply, ControlError>>,
    pub(crate) calls: Vec<Request>,
}

#[cfg(test)]
impl MockBackend {
    pub(crate) fn new(responses: Vec<Result<Reply, ControlError>>) -> Self {
        Self {
            responses: responses.into(),
            calls: Vec::new(),
        }
    }
}

#[cfg(test)]
impl Backend for MockBackend {
    fn call(&mut self, req: &Request) -> Result<Reply, ControlError> {
        self.calls.push(req.clone());
        self.responses
            .pop_front()
            .unwrap_or(Err("mock exhausted".into()))
    }
}

#[cfg(test)]
#[path = "tools_test.rs"]
mod tests;
