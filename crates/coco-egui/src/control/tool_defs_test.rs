use super::*;

const EXPECTED_NAMES: &[&str] = &[
    "list_vms",
    "start_vm",
    "screen_text",
    "screenshot",
    "type_text",
    "enter_basic",
    "press_keys",
    "joystick",
    "insert_disk",
    "eject_disk",
    "reset",
    "set_running",
    "wait",
    "wait_for_text",
    "peek",
    "poke",
];

#[test]
fn every_tool_has_name_description_and_object_schema() {
    let defs = definitions(true);
    assert_eq!(defs.len(), EXPECTED_NAMES.len());
    for def in &defs {
        assert!(def["name"].is_string());
        assert!(def["description"].as_str().is_some_and(|d| !d.is_empty()));
        assert_eq!(def["inputSchema"]["type"], json!("object"));
    }
}

#[test]
fn wait_for_text_bounds_pattern_and_timeout() {
    let defs = definitions(true);
    let wait = defs
        .iter()
        .find(|definition| definition["name"] == "wait_for_text")
        .unwrap();
    let properties = &wait["inputSchema"]["properties"];
    assert_eq!(
        properties["pattern"]["maxLength"],
        json!(crate::control::MAX_WAIT_PATTERN_CHARS)
    );
    assert_eq!(properties["timeout_fields"]["minimum"], json!(1));
    assert_eq!(
        properties["timeout_fields"]["maximum"],
        json!(crate::control::MAX_WAIT_FIELDS)
    );
    assert_eq!(
        wait["inputSchema"]["required"],
        json!(["pattern", "timeout_fields"])
    );
}

#[test]
fn names_match_expected_set_in_order() {
    let defs = definitions(true);
    let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert_eq!(names, EXPECTED_NAMES);
}

#[test]
fn start_vm_requires_vm() {
    let defs = definitions(true);
    let start_vm = defs.iter().find(|d| d["name"] == "start_vm").unwrap();
    assert_eq!(start_vm["inputSchema"]["required"], json!(["vm"]));
}

#[test]
fn list_vms_takes_no_vm_argument() {
    let defs = definitions(true);
    let list_vms = defs.iter().find(|d| d["name"] == "list_vms").unwrap();
    assert_eq!(list_vms["inputSchema"]["properties"], json!({}));
}

#[test]
fn press_keys_description_mentions_named_keys() {
    let defs = definitions(true);
    let press_keys = defs.iter().find(|d| d["name"] == "press_keys").unwrap();
    let description = press_keys["description"].as_str().unwrap();
    assert!(description.contains("ENTER"));
    assert!(description.contains("SHIFT"));
}

#[test]
fn insert_disk_and_eject_disk_cap_drive_at_ui_drives_minus_one() {
    let defs = definitions(true);
    for name in ["insert_disk", "eject_disk"] {
        let def = defs.iter().find(|d| d["name"] == name).unwrap();
        assert_eq!(
            def["inputSchema"]["properties"]["drive"]["maximum"],
            json!(crate::UI_DRIVES - 1),
            "{name} drive maximum must match the manager's exposed drive count"
        );
    }
}

#[test]
fn annotations_classify_read_only_and_mutating_tools() {
    let defs = definitions(true);
    let expected = [
        ("list_vms", true, false, true),
        ("start_vm", false, true, true),
        ("screen_text", true, false, true),
        ("screenshot", true, false, true),
        ("type_text", false, true, false),
        ("enter_basic", false, true, false),
        ("press_keys", false, true, false),
        ("joystick", false, true, true),
        ("insert_disk", false, true, false),
        ("eject_disk", false, true, true),
        ("reset", false, true, false),
        ("set_running", false, true, true),
        ("wait", false, true, false),
        ("wait_for_text", false, true, false),
        ("peek", true, false, true),
        ("poke", false, true, false),
    ];
    for (name, read_only, destructive, idempotent) in expected {
        let annotations = &defs
            .iter()
            .find(|definition| definition["name"] == name)
            .unwrap()["annotations"];
        assert_eq!(annotations["readOnlyHint"], json!(read_only), "{name}");
        assert_eq!(annotations["destructiveHint"], json!(destructive), "{name}");
        assert_eq!(annotations["idempotentHint"], json!(idempotent), "{name}");
        assert_eq!(annotations["openWorldHint"], json!(false), "{name}");
    }
}

#[test]
fn annotations_can_be_omitted_for_older_protocols() {
    for definition in definitions(false) {
        assert!(definition.get("annotations").is_none());
    }
}

#[test]
fn only_tools_with_structured_results_declare_an_output_schema() {
    let defs = definitions(true);
    let with_output: Vec<&str> = defs
        .iter()
        .filter(|d| d.get("outputSchema").is_some())
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        with_output,
        [
            "list_vms",
            "screen_text",
            "enter_basic",
            "wait_for_text",
            "peek"
        ]
    );
    for def in defs.iter().filter(|d| d.get("outputSchema").is_some()) {
        assert_eq!(def["outputSchema"]["type"], json!("object"));
        assert!(def["outputSchema"]["required"].is_array());
    }
}

#[test]
fn screen_tools_declare_the_same_output_schema() {
    let defs = definitions(true);
    let output = |name| {
        defs.iter()
            .find(|definition| definition["name"] == name)
            .unwrap()["outputSchema"]
            .clone()
    };
    assert_eq!(output("wait_for_text"), output("screen_text"));
    assert_eq!(
        output("enter_basic")["properties"]["screen"],
        output("screen_text")
    );
}

#[test]
fn enter_basic_bounds_the_listing_and_states_the_line_limit() {
    let defs = definitions(true);
    let enter_basic = defs
        .iter()
        .find(|definition| definition["name"] == "enter_basic")
        .unwrap();
    assert_eq!(
        enter_basic["inputSchema"]["properties"]["listing"]["maxLength"],
        json!(crate::control::MAX_ENTER_BASIC_CHARS)
    );
    assert_eq!(enter_basic["inputSchema"]["required"], json!(["listing"]));
    let description = enter_basic["description"].as_str().unwrap();
    let line_limit = crate::control::enter_basic::BASIC_LINE_MAX_CHARS.to_string();
    assert!(description.contains(&line_limit));
}

#[test]
fn list_vms_status_enum_names_every_vm_status() {
    let defs = definitions(true);
    let list_vms = defs.iter().find(|d| d["name"] == "list_vms").unwrap();
    let status = &list_vms["outputSchema"]["properties"]["vms"]["items"]["properties"]["status"];
    let names: Vec<&str> = VmStatus::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(status["enum"], json!(names));
}
