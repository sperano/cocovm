use serde_json::json;

use super::*;

#[test]
fn schema_describes_default_clear_result_and_non_idempotent_mutation() {
    let definition = definition(true);

    assert_eq!(definition["inputSchema"]["required"], json!(null));
    assert_eq!(
        definition["inputSchema"]["properties"]["clear"]["default"],
        json!(false)
    );
    assert_eq!(
        definition["outputSchema"]["required"],
        json!(["text", "truncated"])
    );
    assert_eq!(definition["annotations"]["readOnlyHint"], json!(false));
    assert_eq!(definition["annotations"]["destructiveHint"], json!(true));
    assert_eq!(definition["annotations"]["idempotentHint"], json!(false));
    assert!(
        definition["description"]
            .as_str()
            .unwrap()
            .contains("returns the snapshot first")
    );
}
