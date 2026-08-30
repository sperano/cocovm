use super::*;

const EXPECTED_NAMES: &[&str] = &[
    "list_vms",
    "start_vm",
    "screen_text",
    "screenshot",
    "type_text",
    "press_keys",
    "joystick",
    "insert_disk",
    "eject_disk",
    "reset",
    "set_running",
    "wait",
    "peek",
    "poke",
];

#[test]
fn every_tool_has_name_description_and_object_schema() {
    let defs = definitions();
    assert_eq!(defs.len(), EXPECTED_NAMES.len());
    for def in &defs {
        assert!(def["name"].is_string());
        assert!(def["description"].as_str().is_some_and(|d| !d.is_empty()));
        assert_eq!(def["inputSchema"]["type"], json!("object"));
    }
}

#[test]
fn names_match_expected_set_in_order() {
    let defs = definitions();
    let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(names, EXPECTED_NAMES);
}

#[test]
fn start_vm_requires_vm() {
    let defs = definitions();
    let start_vm = defs.iter().find(|d| d["name"] == "start_vm").unwrap();
    assert_eq!(start_vm["inputSchema"]["required"], json!(["vm"]));
}

#[test]
fn list_vms_takes_no_vm_argument() {
    let defs = definitions();
    let list_vms = defs.iter().find(|d| d["name"] == "list_vms").unwrap();
    assert_eq!(list_vms["inputSchema"]["properties"], json!({}));
}

#[test]
fn press_keys_description_mentions_named_keys() {
    let defs = definitions();
    let press_keys = defs.iter().find(|d| d["name"] == "press_keys").unwrap();
    let description = press_keys["description"].as_str().unwrap();
    assert!(description.contains("ENTER"));
    assert!(description.contains("SHIFT"));
}
