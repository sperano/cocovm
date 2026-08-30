//! Name, description, and JSON Schema `inputSchema` for every tool
//! `tools/list` reports. [`crate::control::tools`] dispatches calls to these
//! by name.

use coco_core::joystick::{AXIS_CENTER, AXIS_MAX};
use serde_json::{Value, json};

use super::key_names;
use super::{MAX_HOLD_FIELDS, MAX_PEEK_LEN, MAX_POKE_LEN, MAX_TYPE_TEXT_CHARS, MAX_WAIT_FIELDS};

/// Highest floppy drive index a tool call may name — [`crate::UI_DRIVES`] is
/// the manager's own exposed drive count (not `coco_core::fdc::DRIVE_COUNT`,
/// the FDC's larger addressable maximum).
const MAX_DRIVE: u8 = (crate::UI_DRIVES - 1) as u8;

fn tool(name: &str, description: impl Into<String>, schema: Value) -> Value {
    json!({"name": name, "description": description.into(), "inputSchema": schema})
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    let mut schema = json!({"type": "object", "properties": properties});
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

fn vm_property() -> Value {
    json!({
        "type": "string",
        "description": "Manager slug of the VM to target; omit when only one VM is running."
    })
}

fn list_vms() -> Value {
    tool(
        "list_vms",
        "List every VM the manager knows, with its lifecycle status.",
        object_schema(json!({}), &[]),
    )
}

fn start_vm() -> Value {
    tool(
        "start_vm",
        "Start (or resume) a VM by its manager slug; a no-op if it's already running.",
        object_schema(json!({"vm": vm_property()}), &["vm"]),
    )
}

fn screen_text() -> Value {
    tool(
        "screen_text",
        "Read the VM's text screen as lines, plus the current video mode.",
        object_schema(json!({"vm": vm_property()}), &[]),
    )
}

fn screenshot() -> Value {
    tool(
        "screenshot",
        "Capture the VM's framebuffer as a PNG. GIME graphics modes are 640x240 with \
         non-square pixels.",
        object_schema(json!({"vm": vm_property()}), &[]),
    )
}

fn type_text() -> Value {
    tool(
        "type_text",
        format!(
            "Type text into the VM through the keyboard type-ahead; \"\\n\" or \"\\r\" presses \
             ENTER. Blocks until fully typed; at most {MAX_TYPE_TEXT_CHARS} characters per call."
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "text": {"type": "string", "maxLength": MAX_TYPE_TEXT_CHARS}
            }),
            &["text"],
        ),
    )
}

fn press_keys() -> Value {
    tool(
        "press_keys",
        format!(
            "Hold one or more keys together as a chord (e.g. [\"CTRL\",\"ALT\"] or \
             [\"SHIFT\",\"0\"]), then release them. Accepted names: {}.",
            key_names::describe()
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "keys": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                "hold_fields": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_HOLD_FIELDS,
                    "description": "How long to hold the chord, in video fields (60/s)."
                }
            }),
            &["keys"],
        ),
    )
}

fn joystick() -> Value {
    tool(
        "joystick",
        format!(
            "Set a joystick's axes and/or buttons. Axes range 0..={AXIS_MAX}, {AXIS_CENTER} is \
             center. `release` hands the port back to the host's own input source."
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "stick": {"type": "string", "enum": ["left", "right"]},
                "x": {"type": "integer", "minimum": 0, "maximum": AXIS_MAX},
                "y": {"type": "integer", "minimum": 0, "maximum": AXIS_MAX},
                "button1": {"type": "boolean"},
                "button2": {"type": "boolean"},
                "release": {"type": "boolean"}
            }),
            &["stick"],
        ),
    )
}

fn insert_disk() -> Value {
    tool(
        "insert_disk",
        "Mount a disk image file in a floppy drive.",
        object_schema(
            json!({
                "vm": vm_property(),
                "drive": {"type": "integer", "minimum": 0, "maximum": MAX_DRIVE},
                "path": {
                    "type": "string",
                    "description": "Absolute path on the machine running cocovm."
                }
            }),
            &["drive", "path"],
        ),
    )
}

fn eject_disk() -> Value {
    tool(
        "eject_disk",
        "Remove whatever disk image is mounted in a floppy drive.",
        object_schema(
            json!({"vm": vm_property(), "drive": {"type": "integer", "minimum": 0, "maximum": MAX_DRIVE}}),
            &["drive"],
        ),
    )
}

fn reset() -> Value {
    tool(
        "reset",
        "Reset the VM; `hard` power-cycles it (clears RAM).",
        object_schema(
            json!({"vm": vm_property(), "hard": {"type": "boolean"}}),
            &[],
        ),
    )
}

fn set_running() -> Value {
    tool(
        "set_running",
        "Pause or resume emulation.",
        object_schema(
            json!({"vm": vm_property(), "running": {"type": "boolean"}}),
            &["running"],
        ),
    )
}

fn wait() -> Value {
    tool(
        "wait",
        "Let video fields elapse before replying (60 fields is about 1 second).",
        object_schema(
            json!({
                "vm": vm_property(),
                "fields": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT_FIELDS}
            }),
            &["fields"],
        ),
    )
}

fn peek() -> Value {
    tool(
        "peek",
        "Read bytes from VM memory without side effects.",
        object_schema(
            json!({
                "vm": vm_property(),
                "addr": {"type": "integer", "minimum": 0, "maximum": u16::MAX},
                "len": {"type": "integer", "minimum": 1, "maximum": MAX_PEEK_LEN}
            }),
            &["addr", "len"],
        ),
    )
}

fn poke() -> Value {
    tool(
        "poke",
        "Write bytes to VM memory, with normal bus side effects.",
        object_schema(
            json!({
                "vm": vm_property(),
                "addr": {"type": "integer", "minimum": 0, "maximum": u16::MAX},
                "bytes": {
                    "type": "array",
                    "maxItems": MAX_POKE_LEN,
                    "items": {"type": "integer", "minimum": 0, "maximum": u8::MAX}
                }
            }),
            &["addr", "bytes"],
        ),
    )
}

/// Every tool `tools/list` reports, in the order `tools/call` accepts them.
pub fn definitions() -> Vec<Value> {
    vec![
        list_vms(),
        start_vm(),
        screen_text(),
        screenshot(),
        type_text(),
        press_keys(),
        joystick(),
        insert_disk(),
        eject_disk(),
        reset(),
        set_running(),
        wait(),
        peek(),
        poke(),
    ]
}

#[cfg(test)]
#[path = "tool_defs_test.rs"]
mod tests;
