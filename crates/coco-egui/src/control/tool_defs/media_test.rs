use serde_json::{Value, json};

use super::*;

fn input_schema(definition: Value) -> Value {
    definition["inputSchema"].clone()
}

#[test]
fn drive_tools_cap_the_drive_at_each_device_count() {
    let cases = [
        (insert_vhd(true), crate::UI_DRIVES - 1),
        (eject_vhd(true), crate::UI_DRIVES - 1),
        (insert_drivewire_disk(true), MAX_DRIVEWIRE_DRIVE),
        (eject_drivewire_disk(true), MAX_DRIVEWIRE_DRIVE),
    ];
    for (definition, max) in cases {
        let name = definition["name"].clone();
        let drive = &input_schema(definition)["properties"]["drive"];
        assert_eq!(drive["minimum"], json!(0), "{name}");
        assert_eq!(drive["maximum"], json!(max), "{name}");
    }
    assert_eq!(MAX_DRIVEWIRE_DRIVE, coco_core::drivewire::DRIVE_COUNT - 1);
}

#[test]
fn insert_tools_require_a_path() {
    for definition in [
        insert_tape(true),
        insert_vhd(true),
        insert_drivewire_disk(true),
    ] {
        let name = definition["name"].clone();
        let schema = input_schema(definition);
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path")),
            "{name}"
        );
        assert_eq!(schema["properties"]["path"]["type"], json!("string"));
    }
}

#[test]
fn tape_tools_take_only_an_optional_vm() {
    for definition in [eject_tape(true), rewind_tape(true)] {
        let schema = input_schema(definition);
        assert_eq!(schema["properties"], json!({"vm": vm_property()}));
        assert!(schema.get("required").is_none());
    }
}

#[test]
fn mpi_slots_are_one_based_up_to_the_slot_count() {
    let slot_schemas = [
        input_schema(insert_cartridge(true))["properties"]["slot"].clone(),
        input_schema(remove_cartridge(true))["properties"]["slot"].clone(),
        input_schema(set_mpi_switch(true))["properties"]["slot"].clone(),
    ];
    for slot in slot_schemas {
        assert_eq!(slot["minimum"], json!(1));
        assert_eq!(slot["maximum"], json!(MPI_SLOT_COUNT));
    }
    assert_eq!(
        input_schema(set_mpi_switch(true))["required"],
        json!(["slot"])
    );
    assert!(
        input_schema(remove_cartridge(true))
            .get("required")
            .is_none()
    );
}

#[test]
fn insert_cartridge_accepts_port_and_slot_kinds() {
    let schema = input_schema(insert_cartridge(true));
    assert_eq!(schema["required"], json!(["cartridge"]));
    let cartridge = &schema["properties"]["cartridge"];
    let kinds = cartridge["properties"]["kind"]["enum"].as_array().unwrap();
    for kind in CARTRIDGE_KINDS.iter().chain(SLOT_KINDS) {
        assert_eq!(
            kinds.iter().filter(|k| *k == kind).count(),
            1,
            "{kind} listed once"
        );
    }
    let slots = &cartridge["properties"]["slots"];
    assert_eq!(slots["minItems"], json!(MPI_SLOT_COUNT));
    assert_eq!(slots["maxItems"], json!(MPI_SLOT_COUNT));
}
