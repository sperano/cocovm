use serde_json::{Value, json};

use super::{DESTRUCTIVE, object_schema, tool, vm_property};

pub(super) fn definition(include_annotations: bool) -> Value {
    let mut input = object_schema(
        json!({
            "vm": vm_property(),
            "path": {
                "type": "string",
                "description": "Host path to read. Provide exactly one of `path` or `bytes`."
            },
            "bytes": {
                "type": "string",
                "description": "Base64-encoded source bytes. Provide exactly one of `path` or `bytes`."
            },
            "address": {
                "type": "integer",
                "minimum": 0,
                "maximum": u16::MAX,
                "description": "When present, treat the source as raw bytes and write them from this logical address. When omitted, parse a canonical DECB binary."
            },
            "exec": {
                "type": "boolean",
                "default": false,
                "description": "After loading, jump to the DECB execution address, or to `address` for raw bytes."
            }
        }),
        &[],
    );
    input["oneOf"] = json!([
        {"required": ["path"], "not": {"required": ["bytes"]}},
        {"required": ["bytes"], "not": {"required": ["path"]}}
    ]);
    tool(
        "load_binary",
        "Load a canonical DECB binary or raw bytes into VM memory through the logical bus. The complete DECB file is validated before memory changes. Raw and DECB writes wrap past $FFFF. Overlapping DECB segments apply in file order.",
        input,
        DESTRUCTIVE,
        include_annotations,
    )
}
