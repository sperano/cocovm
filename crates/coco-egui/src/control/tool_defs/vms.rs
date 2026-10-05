//! Definitions of the VM lifecycle tools (`list_vms`, `start_vm`, `stop_vm`,
//! `suspend_vm`) and `list_vms`'s per-VM output schema.

use coco_core::MachineVariant;
use serde_json::{Value, json};

use super::{
    DESTRUCTIVE_IDEMPOTENT, READ_ONLY, object_schema, tool, tool_with_output, vm_property,
};
use crate::MPI_SLOT_COUNT;
use crate::control::protocol::{Cpu, VmStatus, model_id};
use crate::machine_def::DosRom;

/// `kind` tags of a direct-port [`crate::machine_def::CartridgeDTO`].
pub(super) const CARTRIDGE_KINDS: &[&str] = &[
    "none",
    "fd502",
    "rompak",
    "banked_rompak",
    "rtc",
    "rs232",
    "gmc",
    "orch90",
    "ssc",
    "cocomax",
    "mpi",
];
/// `kind` tags of a MultiPak [`crate::machine_def::SlotDTO`].
pub(super) const SLOT_KINDS: &[&str] = &[
    "empty",
    "fd502",
    "rompak",
    "banked_rompak",
    "rtc",
    "rs232",
    "gmc",
    "orch90",
    "ssc",
    "cocomax",
];
/// `kind` tags of a [`crate::machine_def::RS232EndpointDTO`].
pub(super) const RS232_ENDPOINT_KINDS: &[&str] = &["loopback", "tcp", "pty"];
/// Every [`DosRom`], for the `dos_rom` enum.
const DOS_ROMS: [DosRom; 2] = [DosRom::DiskBasic, DosRom::HdbDosDw3];

pub(super) fn list_vms(include_annotations: bool) -> Value {
    tool_with_output(
        "list_vms",
        "List every VM the manager knows: lifecycle status, model, RAM, CPU, cartridge, and \
         mounted media.",
        object_schema(json!({}), &[]),
        object_schema(
            json!({"vms": {"type": "array", "items": vm_schema()}}),
            &["vms"],
        ),
        READ_ONLY,
        include_annotations,
    )
}

fn vm_schema() -> Value {
    let statuses: Vec<&str> = VmStatus::ALL.iter().map(|s| s.as_str()).collect();
    let models: Vec<&str> = MachineVariant::ALL.iter().map(|&v| model_id(v)).collect();
    let cpus: Vec<&str> = Cpu::ALL.iter().map(|c| c.as_str()).collect();
    object_schema(
        json!({
            "slug": {"type": "string"},
            "name": {"type": "string"},
            "status": {"type": "string", "enum": statuses},
            "model": {"type": "string", "enum": models},
            "ram_kib": {"type": "integer", "minimum": 1},
            "cpu": {"type": "string", "enum": cpus},
            "cartridge": cartridge_schema(),
            "media": media_schema()
        }),
        &[
            "slug",
            "name",
            "status",
            "model",
            "ram_kib",
            "cpu",
            "cartridge",
        ],
    )
}

/// Properties a cartridge-port occupant and a MultiPak slot share; which
/// appear depends on `kind`.
fn peripheral_properties(kinds: &[&str]) -> Value {
    let dos_roms: Vec<Value> = DOS_ROMS
        .iter()
        .map(|rom| serde_json::to_value(rom).expect("a DosRom always serializes"))
        .collect();
    json!({
        "kind": {"type": "string", "enum": kinds},
        "path": {"type": "string", "description": "ROM image of a rompak, banked_rompak, or gmc."},
        "dos_rom": {"type": "string", "enum": dos_roms},
        "endpoint": object_schema(
            json!({
                "kind": {"type": "string", "enum": RS232_ENDPOINT_KINDS},
                "listen": {"type": "string"}
            }),
            &["kind"],
        )
    })
}

fn cartridge_schema() -> Value {
    let mut properties = peripheral_properties(CARTRIDGE_KINDS);
    properties["slots"] = json!({
        "type": "array",
        "maxItems": MPI_SLOT_COUNT,
        "items": object_schema(peripheral_properties(SLOT_KINDS), &["kind"])
    });
    properties["switch"] = json!({
        "type": "integer",
        "minimum": 1,
        "maximum": MPI_SLOT_COUNT,
        "description": "MultiPak front-panel slot (1-based) selected at power-on."
    });
    let mut schema = object_schema(properties, &["kind"]);
    schema["description"] = json!(
        "Cartridge-port occupant from the VM's definition, in its [peripherals] format. A \
         cartridge swapped from a running VM's menu is not reflected."
    );
    schema
}

fn media_schema() -> Value {
    let path_or_null = json!({"type": ["string", "null"]});
    let drives = |description: &str| json!({"type": "array", "items": path_or_null, "description": description});
    let mut schema = object_schema(
        json!({
            "disks": drives("FD-502 floppies, indexed like insert_disk's drive."),
            "vhds": drives("Virtual hard disks."),
            "drivewire": drives("DriveWire disks; all null when DriveWire is off."),
            "tape": path_or_null
        }),
        &["disks", "vhds", "drivewire", "tape"],
    );
    schema["description"] = json!(
        "Image paths by drive: the live VM's mounts, or for a powered-off VM what starting it \
         mounts. Absent for a suspended VM whose window is closed."
    );
    schema
}

pub(super) fn start_vm(include_annotations: bool) -> Value {
    tool(
        "start_vm",
        "Start (or resume) a VM by its manager slug; a no-op if it's already running.",
        object_schema(json!({"vm": vm_property()}), &["vm"]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

pub(super) fn stop_vm(include_annotations: bool) -> Value {
    tool(
        "stop_vm",
        "Power a VM off, like the manager's Stop: modified floppies and tape are written back to \
         their files, then the VM is shut down. A suspended VM's saved state is discarded. If a \
         write-back fails, the VM still powers off and the call returns an error naming the \
         file and the state the VM ended in. A no-op if the VM is already powered off.",
        object_schema(json!({"vm": vm_property()}), &["vm"]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

pub(super) fn suspend_vm(include_annotations: bool) -> Value {
    tool(
        "suspend_vm",
        "Suspend a running VM, like the manager's Suspend: modified floppies and tape are \
         written back, the machine state is saved to disk, and emulation pauses; start_vm \
         resumes it. If a write-back or the save fails, the VM keeps running and the call \
         returns an error. A no-op if the VM is already suspended.",
        object_schema(json!({"vm": vm_property()}), &["vm"]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

#[cfg(test)]
#[path = "vms_test.rs"]
mod tests;
