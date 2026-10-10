use serde_json::{Value, json};

use super::{DESTRUCTIVE, object_schema, tool, vm_property};

const FIRST_QUICK_SLOT: usize = 1;

fn state_schema() -> Value {
    let mut input = object_schema(
        json!({
            "vm": vm_property(),
            "path": {
                "type": "string",
                "description": "Host path to a .ccstate file. Provide exactly one of `path` or `slot`."
            },
            "slot": {
                "type": "integer",
                "minimum": FIRST_QUICK_SLOT,
                "maximum": crate::save_state::QUICK_SLOTS,
                "description": "One-based quick-state slot. Provide exactly one of `path` or `slot`."
            }
        }),
        &[],
    );
    input["oneOf"] = json!([
        {"required": ["path"], "not": {"required": ["slot"]}},
        {"required": ["slot"], "not": {"required": ["path"]}}
    ]);
    input
}

pub(super) fn save_state(include_annotations: bool) -> Value {
    tool(
        "save_state",
        "Save the VM's complete state to a .ccstate host path or an app quick-state slot. Dirty writable media is written back first; a write-back failure aborts the save.",
        state_schema(),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn load_state(include_annotations: bool) -> Value {
    tool(
        "load_state",
        "Replace the VM with the complete state from a .ccstate host path or an app quick-state slot.",
        state_schema(),
        DESTRUCTIVE,
        include_annotations,
    )
}
