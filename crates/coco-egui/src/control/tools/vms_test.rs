use coco_core::{MachineVariant, MemorySize};
use serde_json::json;

use super::*;
use crate::control::protocol::{ControlError, Cpu, VmStatus};
use crate::control::tools::{MockBackend, call};
use crate::machine_def::{DosRom, RS232EndpointDTO};

fn call_params(name: &str, arguments: Value) -> Value {
    json!({"name": name, "arguments": arguments})
}

fn text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

#[test]
fn list_vms_text_reports_hardware_cartridge_and_media() {
    let vms = sample_vms();
    assert_eq!(
        format_vm(&vms[0]),
        "vm0 — VM 0 (running)\n  CoCo 3, 512K RAM, MC6809\n  cartridge: FD-502 (Disk BASIC \
         1.1)\n  media: disk0 /disks/a.dsk, tape /tapes/t.cas"
    );
    assert_eq!(
        format_vm(&vms[2]),
        "vm2 — VM 2 (powered_off)\n  CoCo 1, 16K RAM, MC6809\n  cartridge: none\n  media: none"
    );
}

#[test]
fn list_vms_text_lists_multipak_slots_and_unknown_media() {
    let text = format_vm(&sample_vms()[1]);
    assert!(
        text.contains(
            "cartridge: MultiPak, switch on slot 1; slots: 1 FD-502 (HDB-DOS DW3), 2 ROM Pak \
             /roms/pak.rom, 3 empty, 4 Deluxe RS-232 Pak"
        ),
        "{text}"
    );
    assert!(text.contains("media: unknown until resumed"), "{text}");
}

#[test]
fn media_text_names_every_kind_of_drive() {
    let media = VmMedia {
        disks: [None, Some("/d1.dsk".into())],
        vhds: [Some("/h0.vhd".into()), None],
        drivewire: [None, None, None, Some("/dw3.dsk".into())],
        tape: None,
    };
    assert_eq!(
        media_text(&media),
        "disk1 /d1.dsk, vhd0 /h0.vhd, dw3 /dw3.dsk"
    );
}

#[test]
fn stop_and_suspend_send_their_action_for_the_named_vm() {
    let cases = [
        ("stop_vm", Action::StopVm, "Stopped."),
        ("suspend_vm", Action::SuspendVm, "Suspended."),
    ];
    for (name, action, message) in cases {
        let mut mock = MockBackend::new(vec![Ok(Reply::Done)]);
        let result = call(&mut mock, call_params(name, json!({"vm": "coco3"})), true).unwrap();
        assert_eq!(text(&result), message, "{name}");
        assert_eq!(
            mock.calls,
            [Request {
                vm: Some("coco3".into()),
                action
            }],
            "{name}"
        );
    }
}

#[test]
fn stop_and_suspend_require_a_vm() {
    for name in ["stop_vm", "suspend_vm"] {
        let mut mock = MockBackend::new(vec![]);
        let error = call(&mut mock, call_params(name, json!({})), true).unwrap_err();
        assert_eq!(
            error.code,
            crate::control::jsonrpc::INVALID_PARAMS,
            "{name}"
        );
        assert!(mock.calls.is_empty(), "{name}");
    }
}

#[test]
fn a_failed_stop_is_an_error_result_carrying_the_message() {
    const FAILURE: &str = "could not save /disks/a.dsk: read-only\nVM 'coco3' is now powered_off.";
    let mut mock = MockBackend::new(vec![Err(ControlError::from(FAILURE))]);
    let result = call(
        &mut mock,
        call_params("stop_vm", json!({"vm": "coco3"})),
        true,
    )
    .unwrap();
    assert_eq!(result["isError"], json!(true));
    assert_eq!(text(&result), FAILURE);
}

/// One VM in each status, covering a direct-port cartridge, a MultiPak, and
/// known and unknown media. `tools`' own tests reuse it.
pub(in crate::control) fn sample_vms() -> Vec<VmInfo> {
    let running = VmInfo {
        slug: "vm0".into(),
        name: "VM 0".into(),
        status: VmStatus::Running,
        model: MachineVariant::Coco3,
        ram: MemorySize::K512,
        cpu: Cpu::MC6809,
        cartridge: CartridgeDTO::FD502 {
            dos_rom: DosRom::DiskBasic,
        },
        media: Some(VmMedia {
            disks: [Some(PathBuf::from("/disks/a.dsk")), None],
            tape: Some(PathBuf::from("/tapes/t.cas")),
            ..VmMedia::default()
        }),
    };
    let suspended = VmInfo {
        slug: "vm1".into(),
        name: "VM 1".into(),
        status: VmStatus::Suspended,
        model: MachineVariant::Coco2,
        ram: MemorySize::K64,
        cpu: Cpu::MC6809,
        cartridge: CartridgeDTO::MPI {
            slots: [
                SlotDTO::FD502 {
                    dos_rom: DosRom::HdbDosDw3,
                },
                SlotDTO::ROMPak {
                    path: "/roms/pak.rom".into(),
                },
                SlotDTO::Empty,
                SlotDTO::RS232 {
                    endpoint: RS232EndpointDTO::TCP {
                        listen: "127.0.0.1:6551".into(),
                    },
                },
            ],
            switch: 1,
        },
        media: None,
    };
    let powered_off = VmInfo {
        slug: "vm2".into(),
        name: "VM 2".into(),
        status: VmStatus::PoweredOff,
        model: MachineVariant::Coco1,
        ram: MemorySize::K16,
        cpu: Cpu::MC6809,
        cartridge: CartridgeDTO::None,
        media: Some(VmMedia::default()),
    };
    vec![running, suspended, powered_off]
}

/// [`sample_vms`] as `list_vms`'s `structuredContent`.
pub(in crate::control::tools) fn sample_vms_json() -> Value {
    let no_drives = Value::Array(vec![Value::Null; crate::UI_DRIVES]);
    let no_dw = Value::Array(vec![Value::Null; coco_core::drivewire::DRIVE_COUNT]);
    json!({"vms": [
        {
            "slug": "vm0", "name": "VM 0", "status": "running",
            "model": "coco3", "ram_kib": 512, "cpu": "MC6809",
            "cartridge": {"kind": "fd502", "dos_rom": "disk_basic"},
            "media": {
                "disks": ["/disks/a.dsk", null],
                "vhds": no_drives,
                "drivewire": no_dw,
                "tape": "/tapes/t.cas"
            }
        },
        {
            "slug": "vm1", "name": "VM 1", "status": "suspended",
            "model": "coco2", "ram_kib": 64, "cpu": "MC6809",
            "cartridge": {
                "kind": "mpi",
                "slots": [
                    {"kind": "fd502", "dos_rom": "hdb_dos_dw3"},
                    {"kind": "rompak", "path": "/roms/pak.rom"},
                    {"kind": "empty"},
                    {"kind": "rs232", "endpoint": {"kind": "tcp", "listen": "127.0.0.1:6551"}}
                ],
                "switch": 1
            }
        },
        {
            "slug": "vm2", "name": "VM 2", "status": "powered_off",
            "model": "coco1", "ram_kib": 16, "cpu": "MC6809",
            "cartridge": {"kind": "none"},
            "media": {"disks": no_drives, "vhds": no_drives, "drivewire": no_dw, "tape": null}
        }
    ]})
}
