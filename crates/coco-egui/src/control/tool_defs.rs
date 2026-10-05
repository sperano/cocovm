//! Name, description, and JSON Schema `inputSchema` for every tool
//! `tools/list` reports, plus an `outputSchema` for the tools whose result
//! has structure. [`crate::control::tools`] dispatches calls to these by
//! name, and drops `outputSchema` for clients older than protocol 2025-06-18.

use coco_core::joystick::{AXIS_CENTER, AXIS_MAX};
use serde_json::{Value, json};

use super::enter_basic::BASIC_LINE_MAX_CHARS;
use super::key_names;
use super::protocol::VmStatus;
use super::{
    MAX_ENTER_BASIC_CHARS, MAX_HOLD_FIELDS, MAX_PEEK_LEN, MAX_POKE_LEN, MAX_TYPE_TEXT_CHARS,
    MAX_WAIT_FIELDS, MAX_WAIT_PATTERN_CHARS,
};

/// Highest floppy drive index a tool call may name — [`crate::UI_DRIVES`] is
/// the manager's own exposed drive count (not `coco_core::fdc::DRIVE_COUNT`,
/// the FDC's larger addressable maximum).
const MAX_DRIVE: u8 = (crate::UI_DRIVES - 1) as u8;

#[derive(Clone, Copy)]
struct ToolAnnotations {
    read_only: bool,
    destructive: bool,
    idempotent: bool,
}

const READ_ONLY: ToolAnnotations = ToolAnnotations {
    read_only: true,
    destructive: false,
    idempotent: true,
};
const DESTRUCTIVE_IDEMPOTENT: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: true,
    idempotent: true,
};
const DESTRUCTIVE: ToolAnnotations = ToolAnnotations {
    read_only: false,
    destructive: true,
    idempotent: false,
};

fn tool(
    name: &str,
    description: impl Into<String>,
    schema: Value,
    annotations: ToolAnnotations,
    include_annotations: bool,
) -> Value {
    let mut definition = json!({
        "name": name,
        "description": description.into(),
        "inputSchema": schema,
    });
    if include_annotations {
        definition["annotations"] = json!({
            "readOnlyHint": annotations.read_only,
            "destructiveHint": annotations.destructive,
            "idempotentHint": annotations.idempotent,
            "openWorldHint": false,
        });
    }
    definition
}

/// [`tool`] with an `outputSchema` its `structuredContent` conforms to.
fn tool_with_output(
    name: &str,
    description: impl Into<String>,
    input: Value,
    output: Value,
    annotations: ToolAnnotations,
    include_annotations: bool,
) -> Value {
    let mut def = tool(name, description, input, annotations, include_annotations);
    def["outputSchema"] = output;
    def
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

fn byte_schema() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": u8::MAX})
}

fn address_schema() -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": u16::MAX})
}

fn list_vms(include_annotations: bool) -> Value {
    let statuses: Vec<&str> = VmStatus::ALL.iter().map(|s| s.as_str()).collect();
    let vm = object_schema(
        json!({
            "slug": {"type": "string"},
            "name": {"type": "string"},
            "status": {"type": "string", "enum": statuses}
        }),
        &["slug", "name", "status"],
    );
    tool_with_output(
        "list_vms",
        "List every VM the manager knows, with its lifecycle status.",
        object_schema(json!({}), &[]),
        object_schema(json!({"vms": {"type": "array", "items": vm}}), &["vms"]),
        READ_ONLY,
        include_annotations,
    )
}

fn start_vm(include_annotations: bool) -> Value {
    tool(
        "start_vm",
        "Start (or resume) a VM by its manager slug; a no-op if it's already running.",
        object_schema(json!({"vm": vm_property()}), &["vm"]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

fn screen_text(include_annotations: bool) -> Value {
    tool_with_output(
        "screen_text",
        "Read the VM's text screen as lines, plus the current video mode and the 0-based \
         row/column where BASIC's next character lands (32-column VDG and WIDTH 40/80 \
         screens). Graphics modes have no text buffer.",
        object_schema(json!({"vm": vm_property()}), &[]),
        screen_schema(),
        READ_ONLY,
        include_annotations,
    )
}

fn screenshot(include_annotations: bool) -> Value {
    tool(
        "screenshot",
        "Capture the VM's framebuffer as a PNG. GIME graphics modes are 640x240 with \
         non-square pixels.",
        object_schema(json!({"vm": vm_property()}), &[]),
        READ_ONLY,
        include_annotations,
    )
}

fn type_text(include_annotations: bool) -> Value {
    tool(
        "type_text",
        format!(
            "Type text into the VM through the keyboard type-ahead; \"\\n\" or \"\\r\" presses \
             ENTER. Blocks until fully typed; at most {MAX_TYPE_TEXT_CHARS} characters per call. \
             Accepts letters, digits, space, and the punctuation on the CoCo keyboard; a \
             character with no CoCo key (such as [ {{ ~, tab, or non-ASCII) fails the whole \
             call with an error naming it, and nothing is typed."
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "text": {"type": "string", "maxLength": MAX_TYPE_TEXT_CHARS}
            }),
            &["text"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

fn enter_basic(include_annotations: bool) -> Value {
    tool_with_output(
        "enter_basic",
        format!(
            "Type a multi-line BASIC listing at the BASIC prompt, one line at a time (ENTER \
             after each; blank lines are skipped). With \"new\": true, types NEW first. Takes \
             about 0.1 s per character, so a long listing runs for minutes. The whole listing \
             is checked first: at most {MAX_ENTER_BASIC_CHARS} characters, at most \
             {BASIC_LINE_MAX_CHARS} per line, and only characters on the CoCo keyboard; \
             otherwise nothing is typed. Stops at the first line BASIC answers with an error \
             (such as ?SN ERROR or ?OM ERROR), and reports that line, the error, and the \
             screen; an error printed later, such as by a long-running RUN, is not caught. \
             On success, returns the line count and the final screen."
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "listing": {"type": "string", "maxLength": MAX_ENTER_BASIC_CHARS},
                "new": {
                    "type": "boolean",
                    "description": "Type NEW first, erasing the program in memory."
                }
            }),
            &["listing"],
        ),
        object_schema(
            json!({
                "lines": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Listing lines typed, not counting NEW."
                },
                "screen": screen_schema()
            }),
            &["lines", "screen"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

fn press_keys(include_annotations: bool) -> Value {
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
        DESTRUCTIVE,
        include_annotations,
    )
}

fn joystick(include_annotations: bool) -> Value {
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
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

fn insert_disk(include_annotations: bool) -> Value {
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
        DESTRUCTIVE,
        include_annotations,
    )
}

fn eject_disk(include_annotations: bool) -> Value {
    tool(
        "eject_disk",
        "Remove whatever disk image is mounted in a floppy drive.",
        object_schema(
            json!({"vm": vm_property(), "drive": {"type": "integer", "minimum": 0, "maximum": MAX_DRIVE}}),
            &["drive"],
        ),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

fn reset(include_annotations: bool) -> Value {
    tool(
        "reset",
        "Reset the VM; `hard` power-cycles it (clears RAM).",
        object_schema(
            json!({"vm": vm_property(), "hard": {"type": "boolean"}}),
            &[],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

fn set_running(include_annotations: bool) -> Value {
    tool(
        "set_running",
        "Pause or resume emulation.",
        object_schema(
            json!({"vm": vm_property(), "running": {"type": "boolean"}}),
            &["running"],
        ),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

fn wait(include_annotations: bool) -> Value {
    tool(
        "wait",
        "Let video fields elapse before replying (60 fields is about 1 second). Fails if the \
         VM is or becomes paused.",
        object_schema(
            json!({
                "vm": vm_property(),
                "fields": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT_FIELDS}
            }),
            &["fields"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

fn wait_for_text(include_annotations: bool) -> Value {
    tool_with_output(
        "wait_for_text",
        "Wait until decoded screen text matches a literal string or regular expression. Returns \
         the matching screen, video mode, and cursor. On timeout, or if the VM is or becomes \
         paused before a match, returns an error with the last screen state.",
        object_schema(
            json!({
                "vm": vm_property(),
                "pattern": {"type": "string", "maxLength": MAX_WAIT_PATTERN_CHARS},
                "regex": {"type": "boolean"},
                "timeout_fields": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_WAIT_FIELDS,
                    "description": "Maximum wait in video fields (60/s)."
                }
            }),
            &["pattern", "timeout_fields"],
        ),
        screen_schema(),
        DESTRUCTIVE,
        include_annotations,
    )
}

/// `screen_text`'s optional `cursor`, absent when the text says "unknown".
fn cursor_schema() -> Value {
    let mut schema = object_schema(
        json!({
            "row": {"type": "integer", "minimum": 0},
            "col": {"type": "integer", "minimum": 0}
        }),
        &["row", "col"],
    );
    schema["description"] = json!(
        "0-based row/column where BASIC's next character lands; absent in graphics \
         modes or when BASIC is not driving the screen."
    );
    schema
}

fn screen_schema() -> Value {
    object_schema(
        json!({
            "lines": {
                "type": "array",
                "items": {"type": "string"},
                "description": "One string per screen row, top to bottom."
            },
            "mode": {
                "type": "string",
                "description": "Video-mode summary, with the text buffer's base address."
            },
            "cursor": cursor_schema()
        }),
        &["lines", "mode"],
    )
}

fn peek(include_annotations: bool) -> Value {
    tool_with_output(
        "peek",
        "Read bytes from VM memory without side effects.",
        object_schema(
            json!({
                "vm": vm_property(),
                "addr": address_schema(),
                "len": {"type": "integer", "minimum": 1, "maximum": MAX_PEEK_LEN}
            }),
            &["addr", "len"],
        ),
        object_schema(
            json!({
                "addr": address_schema(),
                "bytes": {
                    "type": "array",
                    "maxItems": MAX_PEEK_LEN,
                    "items": byte_schema(),
                    "description": "Bytes read from `addr` upward, wrapping past $FFFF."
                }
            }),
            &["addr", "bytes"],
        ),
        READ_ONLY,
        include_annotations,
    )
}

fn poke(include_annotations: bool) -> Value {
    tool(
        "poke",
        "Write bytes to VM memory, with normal bus side effects.",
        object_schema(
            json!({
                "vm": vm_property(),
                "addr": address_schema(),
                "bytes": {
                    "type": "array",
                    "maxItems": MAX_POKE_LEN,
                    "items": byte_schema()
                }
            }),
            &["addr", "bytes"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

/// Every tool `tools/list` reports, in the order `tools/call` accepts them.
pub fn definitions(include_annotations: bool) -> Vec<Value> {
    vec![
        list_vms(include_annotations),
        start_vm(include_annotations),
        screen_text(include_annotations),
        screenshot(include_annotations),
        type_text(include_annotations),
        enter_basic(include_annotations),
        press_keys(include_annotations),
        joystick(include_annotations),
        insert_disk(include_annotations),
        eject_disk(include_annotations),
        reset(include_annotations),
        set_running(include_annotations),
        wait(include_annotations),
        wait_for_text(include_annotations),
        peek(include_annotations),
        poke(include_annotations),
    ]
}

#[cfg(test)]
#[path = "tool_defs_test.rs"]
mod tests;
