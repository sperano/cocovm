use serde_json::{Value, json};

use super::{DESTRUCTIVE, object_schema, tool_with_output, vm_property};

pub(super) fn definition(include_annotations: bool) -> Value {
    tool_with_output(
        "read_printer",
        format!(
            "Read text captured from LLIST and PRINT #-2. The capture keeps the newest {} KiB, \
             converts CR LF and lone CR line endings to LF, and replaces invalid UTF-8. If \
             `clear` is true, \
             the tool returns the snapshot first, and then clears the capture and its \
             truncation marker.",
            coco_core::printer::PRINTER_CAPTURE_CAPACITY / 1024
        ),
        object_schema(
            json!({
                "vm": vm_property(),
                "clear": {
                    "type": "boolean",
                    "default": false,
                    "description": "Clear captured output after returning this snapshot."
                }
            }),
            &[],
        ),
        object_schema(
            json!({
                "text": {"type": "string"},
                "truncated": {
                    "type": "boolean",
                    "description": "True when older bytes were discarded before this text."
                }
            }),
            &["text", "truncated"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

#[cfg(test)]
#[path = "printer_test.rs"]
mod tests;
