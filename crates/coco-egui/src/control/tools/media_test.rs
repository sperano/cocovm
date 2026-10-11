use serde_json::{Value, json};

use super::*;
use crate::control::Reply;
use crate::control::tools::{MockBackend, call};
use crate::machine_def::{DosRom, RS232EndpointDTO};

const STRUCTURED: bool = true;

fn call_tool(mock: &mut MockBackend, name: &str, arguments: Value) -> Result<Value, RpcError> {
    call(
        mock,
        json!({"name": name, "arguments": arguments}),
        STRUCTURED,
    )
}

/// The one request `name` sends for `arguments`, which must succeed.
fn sent(name: &str, arguments: Value) -> Request {
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(&mut mock, name, arguments).unwrap();
    assert_eq!(result["isError"], json!(false), "{name}: {result}");
    assert_eq!(mock.calls.len(), 1);
    mock.calls.remove(0)
}

/// The `INVALID_PARAMS` error `name` answers `arguments` with, sending nothing.
fn rejected(name: &str, arguments: Value) -> RpcError {
    let mut mock = MockBackend::new(vec![]);
    let error = call_tool(&mut mock, name, arguments).unwrap_err();
    assert_eq!(error.code, INVALID_PARAMS, "{name}");
    assert!(mock.calls.is_empty(), "{name} must not reach the app");
    error
}

#[test]
fn tape_tools_map_to_their_actions() {
    let insert = sent("insert_tape", json!({"vm": "demo", "path": "/t/game.cas"}));
    assert_eq!(insert.vm.as_deref(), Some("demo"));
    assert_eq!(
        insert.action,
        Action::InsertTape {
            path: "/t/game.cas".into()
        }
    );
    assert_eq!(sent("eject_tape", json!({})).action, Action::EjectTape);
    assert_eq!(sent("rewind_tape", json!({})).action, Action::RewindTape);
}

#[test]
fn drive_tools_forward_drive_and_path() {
    assert_eq!(
        sent("insert_vhd", json!({"drive": 1, "path": "/h/os9.vhd"})).action,
        Action::InsertVHD {
            drive: 1,
            path: "/h/os9.vhd".into()
        }
    );
    assert_eq!(
        sent("eject_vhd", json!({"drive": 0})).action,
        Action::EjectVHD { drive: 0 }
    );
    assert_eq!(
        sent(
            "insert_drivewire_disk",
            json!({"drive": 3, "path": "/d/x.dsk"})
        )
        .action,
        Action::InsertDriveWireDisk {
            drive: 3,
            path: "/d/x.dsk".into()
        }
    );
    assert_eq!(
        sent("eject_drivewire_disk", json!({"drive": 2})).action,
        Action::EjectDriveWireDisk { drive: 2 }
    );
}

#[test]
fn a_port_cartridge_parses_in_the_definition_format() {
    let request = sent(
        "insert_cartridge",
        json!({"cartridge": {"kind": "fd502", "dos_rom": "hdb_dos_dw3"}}),
    );
    assert_eq!(
        request.action,
        Action::InsertCartridge {
            target: CartridgeTarget::Port(CartridgeDTO::FD502 {
                dos_rom: DosRom::HdbDosDw3
            })
        }
    );
}

#[test]
fn a_whole_multipak_parses_with_its_slots_and_switch() {
    let request = sent(
        "insert_cartridge",
        json!({"cartridge": {
            "kind": "mpi",
            "switch": 1,
            "slots": [
                {"kind": "rompak", "path": "/r/game.ccc"},
                {"kind": "empty"},
                {"kind": "rs232"},
                {"kind": "fd502"}
            ]
        }}),
    );
    assert_eq!(
        request.action,
        Action::InsertCartridge {
            target: CartridgeTarget::Port(CartridgeDTO::MPI {
                slots: [
                    SlotDTO::ROMPak {
                        path: "/r/game.ccc".to_string()
                    },
                    SlotDTO::Empty,
                    SlotDTO::RS232 {
                        endpoint: RS232EndpointDTO::Loopback
                    },
                    SlotDTO::FD502 {
                        dos_rom: DosRom::DiskBasic
                    },
                ],
                switch: 1,
            })
        }
    );
}

#[test]
fn a_slot_cartridge_uses_a_zero_based_slot_index() {
    let request = sent(
        "insert_cartridge",
        json!({"vm": "demo", "slot": MPI_SLOT_COUNT, "cartridge": {"kind": "orch90"}}),
    );
    assert_eq!(
        request.action,
        Action::InsertCartridge {
            target: CartridgeTarget::Slot {
                slot: MPI_SLOT_COUNT - 1,
                occupant: SlotDTO::Orch90
            }
        }
    );
}

#[test]
fn malformed_cartridges_are_rejected_before_reaching_the_app() {
    let invalid = [
        json!({"cartridge": {"kind": "floppy"}}),
        json!({"cartridge": {"kind": "rompak"}}),
        json!({"slot": 1, "cartridge": {"kind": "mpi", "slots": []}}),
        json!({"slot": 1, "cartridge": {"kind": "none"}}),
        json!({"cartridge": {"kind": "mpi", "slots": [{"kind": "empty"}]}}),
        json!({"cartridge": {"kind": "mpi", "switch": 0, "slots": [
            {"kind": "empty"}, {"kind": "empty"}, {"kind": "empty"}, {"kind": "empty"}
        ]}}),
        json!({"cartridge": {"kind": "mpi", "switch": MPI_SLOT_COUNT + 1, "slots": [
            {"kind": "empty"}, {"kind": "empty"}, {"kind": "empty"}, {"kind": "empty"}
        ]}}),
        json!({}),
    ];
    for arguments in invalid {
        rejected("insert_cartridge", arguments);
    }
}

#[test]
fn mpi_slot_numbers_outside_one_to_four_are_rejected() {
    for slot in [0, MPI_SLOT_COUNT + 1] {
        let error = rejected("set_mpi_switch", json!({"slot": slot}));
        assert!(error.message.contains("MultiPak slot"), "{}", error.message);
        rejected("remove_cartridge", json!({"slot": slot}));
        rejected(
            "insert_cartridge",
            json!({"slot": slot, "cartridge": {"kind": "empty"}}),
        );
    }
}

#[test]
fn remove_cartridge_and_set_mpi_switch_convert_slots() {
    assert_eq!(
        sent("remove_cartridge", json!({})).action,
        Action::RemoveCartridge { slot: None }
    );
    assert_eq!(
        sent("remove_cartridge", json!({"slot": 2})).action,
        Action::RemoveCartridge { slot: Some(1) }
    );
    let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
    let result = call_tool(&mut mock, "set_mpi_switch", json!({"slot": 3})).unwrap();
    assert_eq!(mock.calls[0].action, Action::SetMPISwitch { slot: 2 });
    assert_eq!(
        result["content"][0]["text"],
        json!("Moved the MultiPak switch to slot 3.")
    );
}

#[test]
fn app_failures_become_tool_errors() {
    let mut mock = MockBackend::new(vec![Err("no tape is mounted".into())]);
    let result = call_tool(&mut mock, "rewind_tape", json!({})).unwrap();
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["content"][0]["text"], json!("no tape is mounted"));
}
