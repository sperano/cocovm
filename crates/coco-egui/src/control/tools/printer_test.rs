use serde_json::json;

use super::*;
use crate::control::protocol::PrinterOutput;
use crate::control::tools::{MockBackend, STRUCTURED_CONTENT};

#[test]
fn omitted_clear_defaults_false_and_returns_structured_output() {
    let mut backend = MockBackend::new(vec![Ok(Reply::Printer(PrinterOutput {
        text: "HELLO\n".to_string(),
        truncated: false,
    }))]);

    let result = dispatch(&mut backend, json!({"vm": "coco3"})).unwrap();

    assert_eq!(
        backend.calls,
        [Request {
            vm: Some("coco3".to_string()),
            action: Action::ReadPrinter { clear: false },
        }]
    );
    assert_eq!(result["content"][0]["text"], json!("HELLO\n"));
    assert_eq!(
        result[STRUCTURED_CONTENT],
        json!({"text": "HELLO\n", "truncated": false})
    );
}

#[test]
fn clear_is_forwarded_and_truncation_is_visible_in_human_text() {
    let mut backend = MockBackend::new(vec![Ok(Reply::Printer(PrinterOutput {
        text: "newest".to_string(),
        truncated: true,
    }))]);

    let result = dispatch(&mut backend, json!({"clear": true})).unwrap();

    assert_eq!(backend.calls[0].action, Action::ReadPrinter { clear: true });
    assert_eq!(
        result["content"][0]["text"],
        json!("[Earlier printer output was truncated.]\nnewest")
    );
    assert_eq!(result[STRUCTURED_CONTENT]["truncated"], json!(true));
}
