//! Definitions of the media tools beyond floppies: the cassette deck,
//! cartridges and the MultiPak switch, VHDs, and DriveWire disks.

use serde_json::{Value, json};

use super::vms::{CARTRIDGE_KINDS, SLOT_KINDS, cartridge_object_schema};
use super::{
    DESTRUCTIVE, DESTRUCTIVE_IDEMPOTENT, FIRST_MPI_SLOT, MAX_DRIVE, object_schema, tool,
    vm_property,
};
use crate::MPI_SLOT_COUNT;

/// Highest DriveWire drive index a tool call may name.
const MAX_DRIVEWIRE_DRIVE: usize = coco_core::drivewire::DRIVE_COUNT - 1;

fn host_path_property() -> Value {
    json!({
        "type": "string",
        "description": "Absolute path on the machine running cocovm."
    })
}

fn drive_property(max: usize) -> Value {
    json!({"type": "integer", "minimum": 0, "maximum": max})
}

fn mpi_slot_property(description: &str) -> Value {
    json!({
        "type": "integer",
        "minimum": FIRST_MPI_SLOT,
        "maximum": MPI_SLOT_COUNT - 1 + FIRST_MPI_SLOT,
        "description": description
    })
}

pub(super) fn insert_tape(include_annotations: bool) -> Value {
    tool(
        "insert_tape",
        "Mount a cassette image (.cas, or a .wav decoded on insert) in the tape deck, at its \
         start. A recording on the previous tape is first saved to its .cas file; if that save \
         fails, nothing changes.",
        object_schema(
            json!({"vm": vm_property(), "path": host_path_property()}),
            &["path"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn eject_tape(include_annotations: bool) -> Value {
    tool(
        "eject_tape",
        "Remove the tape from the deck. A recording is first saved to its .cas file; if that \
         save fails, the tape stays mounted.",
        object_schema(json!({"vm": vm_property()}), &[]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

pub(super) fn rewind_tape(include_annotations: bool) -> Value {
    tool(
        "rewind_tape",
        "Move the tape back to its start, ending any recording in progress. Fails when no \
         tape is mounted.",
        object_schema(json!({"vm": vm_property()}), &[]),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

/// `insert_cartridge`'s `cartridge`: a port occupant, or with `slot` a
/// MultiPak slot occupant, so `kind` admits both sets.
fn insert_cartridge_property() -> Value {
    let mut kinds: Vec<&str> = CARTRIDGE_KINDS.to_vec();
    kinds.extend(
        SLOT_KINDS
            .iter()
            .filter(|kind| !CARTRIDGE_KINDS.contains(kind)),
    );
    let mut schema = cartridge_object_schema();
    schema["properties"]["kind"]["enum"] = json!(kinds);
    schema["description"] = json!(
        "What to install, in the format of a machine definition's [peripherals].cartridge \
         and of list_vms's cartridge. Without slot, any port kind; \"none\" empties the port. \
         With slot, any kind but \"mpi\"; \"empty\" empties the slot. ROM paths should be \
         absolute; a relative one resolves against the VM's folder, as in its definition."
    );
    schema
}

pub(super) fn insert_cartridge(include_annotations: bool) -> Value {
    tool(
        "insert_cartridge",
        "Replace the cartridge in the port, or in one MultiPak slot, then power-cycle the VM. \
         The old cartridge comes out first: modified floppies in an FD-502 being removed are \
         saved first, and if that save fails, nothing changes. If the new cartridge then fails \
         to load (for example, an unreadable ROM file), the error says so and the port or slot \
         stays empty. A Deluxe RS-232 Pak whose TCP endpoint cannot listen goes in on the \
         loopback endpoint, and the call returns an error saying so. Session only: the VM's \
         definition is unchanged.",
        object_schema(
            json!({
                "vm": vm_property(),
                "cartridge": insert_cartridge_property(),
                "slot": mpi_slot_property(
                    "MultiPak slot (1-based) to fill instead of the port; needs a MultiPak \
                     in the port."
                )
            }),
            &["cartridge"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn remove_cartridge(include_annotations: bool) -> Value {
    tool(
        "remove_cartridge",
        "Empty the cartridge port (a MultiPak comes out with everything in it), or one \
         MultiPak slot, then power-cycle the VM. Modified floppies in an FD-502 being removed \
         are saved first; if that save fails, nothing changes. Session only: the VM's \
         definition is unchanged.",
        object_schema(
            json!({
                "vm": vm_property(),
                "slot": mpi_slot_property(
                    "MultiPak slot (1-based) to empty instead of the whole port."
                )
            }),
            &[],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn set_mpi_switch(include_annotations: bool) -> Value {
    tool(
        "set_mpi_switch",
        "Move the MultiPak's front-panel switch to a slot. The switch selects the slot right \
         away, unless software has selected a slot by writing $FF7F since the last reset; \
         then the switch takes over at the next reset. Fails without a MultiPak.",
        object_schema(
            json!({
                "vm": vm_property(),
                "slot": mpi_slot_property("Front-panel slot number (1-based).")
            }),
            &["slot"],
        ),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

pub(super) fn insert_vhd(include_annotations: bool) -> Value {
    tool(
        "insert_vhd",
        "Mount a virtual hard disk image in a VHD drive. Writes go straight to the file. \
         Session only: the VM's definition is unchanged.",
        object_schema(
            json!({
                "vm": vm_property(),
                "drive": drive_property(MAX_DRIVE.into()),
                "path": host_path_property()
            }),
            &["drive", "path"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn eject_vhd(include_annotations: bool) -> Value {
    tool(
        "eject_vhd",
        "Remove whatever image is mounted in a VHD drive.",
        object_schema(
            json!({"vm": vm_property(), "drive": drive_property(MAX_DRIVE.into())}),
            &["drive"],
        ),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

pub(super) fn insert_drivewire_disk(include_annotations: bool) -> Value {
    tool(
        "insert_drivewire_disk",
        "Mount a disk image in a DriveWire drive. Writes go straight to the file, and no other \
         running VM may mount the same image. Fails when the VM's DriveWire is off. Session \
         only: the VM's definition is unchanged, and a later DriveWire settings change \
         replaces this drive's image only if that drive's own setting changes.",
        object_schema(
            json!({
                "vm": vm_property(),
                "drive": drive_property(MAX_DRIVEWIRE_DRIVE),
                "path": host_path_property()
            }),
            &["drive", "path"],
        ),
        DESTRUCTIVE,
        include_annotations,
    )
}

pub(super) fn eject_drivewire_disk(include_annotations: bool) -> Value {
    tool(
        "eject_drivewire_disk",
        "Remove whatever image is mounted in a DriveWire drive. Fails when the VM's DriveWire \
         is off.",
        object_schema(
            json!({"vm": vm_property(), "drive": drive_property(MAX_DRIVEWIRE_DRIVE)}),
            &["drive"],
        ),
        DESTRUCTIVE_IDEMPOTENT,
        include_annotations,
    )
}

#[cfg(test)]
#[path = "media_test.rs"]
mod tests;
