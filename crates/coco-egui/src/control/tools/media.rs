//! The media tools beyond floppies: the cassette deck (`insert_tape`,
//! `eject_tape`, `rewind_tape`), cartridges (`insert_cartridge`,
//! `remove_cartridge`, `set_mpi_switch`), VHDs (`insert_vhd`, `eject_vhd`),
//! and DriveWire disks (`insert_drivewire_disk`, `eject_drivewire_disk`).

use std::path::PathBuf;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Backend, VmOnly, done, finish, parse_args};
use crate::MPI_SLOT_COUNT;
use crate::control::jsonrpc::{INVALID_PARAMS, RpcError};
use crate::control::protocol::{Action, CartridgeTarget, Request};
use crate::control::tool_defs::FIRST_MPI_SLOT;
use crate::machine_def::{CartridgeDTO, SlotDTO};

const CARTRIDGE_INSERTED: &str = "Inserted the cartridge and power-cycled the VM.";
const CARTRIDGE_REMOVED: &str = "Removed the cartridge and power-cycled the VM.";

/// Send `action` to `vm` and answer `message` on success.
fn run(backend: &mut dyn Backend, vm: Option<String>, action: Action, message: &str) -> Value {
    finish(backend, Request { vm, action }, |reply| {
        done(reply, message)
    })
}

#[derive(Deserialize)]
struct PathArgs {
    #[serde(default)]
    vm: Option<String>,
    path: PathBuf,
}

#[derive(Deserialize)]
struct DriveArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
}

#[derive(Deserialize)]
struct DrivePathArgs {
    #[serde(default)]
    vm: Option<String>,
    drive: usize,
    path: PathBuf,
}

pub(super) fn dispatch_insert_tape(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let PathArgs { vm, path } = parse_args(args)?;
    Ok(run(backend, vm, Action::InsertTape { path }, "Inserted."))
}

pub(super) fn dispatch_eject_tape(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    Ok(run(backend, vm, Action::EjectTape, "Ejected."))
}

pub(super) fn dispatch_rewind_tape(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let VmOnly { vm } = parse_args(args)?;
    Ok(run(backend, vm, Action::RewindTape, "Rewound."))
}

#[derive(Deserialize)]
struct InsertCartridgeArgs {
    #[serde(default)]
    vm: Option<String>,
    cartridge: Value,
    #[serde(default)]
    slot: Option<usize>,
}

pub(super) fn dispatch_insert_cartridge(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let InsertCartridgeArgs {
        vm,
        cartridge,
        slot,
    } = parse_args(args)?;
    let target = match slot {
        Some(slot) => CartridgeTarget::Slot {
            slot: mpi_slot_index(slot)?,
            occupant: parse_cartridge::<SlotDTO>(cartridge)?,
        },
        None => CartridgeTarget::Port(port_cartridge(cartridge)?),
    };
    Ok(run(
        backend,
        vm,
        Action::InsertCartridge { target },
        CARTRIDGE_INSERTED,
    ))
}

/// A port occupant; a MultiPak's `switch` must name one of its slots, as a
/// definition file's must (`machine_def::io::check_mpi_slots`).
fn port_cartridge(cartridge: Value) -> Result<CartridgeDTO, RpcError> {
    let cartridge = parse_cartridge::<CartridgeDTO>(cartridge)?;
    if let CartridgeDTO::MPI { switch, .. } = cartridge {
        mpi_slot_index(switch)?;
    }
    Ok(cartridge)
}

fn parse_cartridge<T: DeserializeOwned>(cartridge: Value) -> Result<T, RpcError> {
    serde_json::from_value(cartridge).map_err(|e| invalid_arguments(format!("cartridge: {e}")))
}

#[derive(Deserialize)]
struct RemoveCartridgeArgs {
    #[serde(default)]
    vm: Option<String>,
    #[serde(default)]
    slot: Option<usize>,
}

pub(super) fn dispatch_remove_cartridge(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let RemoveCartridgeArgs { vm, slot } = parse_args(args)?;
    let slot = slot.map(mpi_slot_index).transpose()?;
    Ok(run(
        backend,
        vm,
        Action::RemoveCartridge { slot },
        CARTRIDGE_REMOVED,
    ))
}

#[derive(Deserialize)]
struct SetMPISwitchArgs {
    #[serde(default)]
    vm: Option<String>,
    slot: usize,
}

pub(super) fn dispatch_set_mpi_switch(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let SetMPISwitchArgs { vm, slot } = parse_args(args)?;
    let action = Action::SetMPISwitch {
        slot: mpi_slot_index(slot)?,
    };
    let message = format!("Moved the MultiPak switch to slot {slot}.");
    Ok(run(backend, vm, action, &message))
}

/// A 1-based MultiPak slot number as the app's 0-based index.
fn mpi_slot_index(slot: usize) -> Result<usize, RpcError> {
    let last = MPI_SLOT_COUNT - 1 + FIRST_MPI_SLOT;
    if (FIRST_MPI_SLOT..=last).contains(&slot) {
        Ok(slot - FIRST_MPI_SLOT)
    } else {
        Err(invalid_arguments(format!(
            "MultiPak slot must be {FIRST_MPI_SLOT}..={last}, got {slot}"
        )))
    }
}

fn invalid_arguments(message: String) -> RpcError {
    RpcError::new(INVALID_PARAMS, format!("invalid arguments: {message}"))
}

pub(super) fn dispatch_insert_vhd(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let DrivePathArgs { vm, drive, path } = parse_args(args)?;
    Ok(run(
        backend,
        vm,
        Action::InsertVHD { drive, path },
        "Inserted.",
    ))
}

pub(super) fn dispatch_eject_vhd(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let DriveArgs { vm, drive } = parse_args(args)?;
    Ok(run(backend, vm, Action::EjectVHD { drive }, "Ejected."))
}

pub(super) fn dispatch_insert_drivewire_disk(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let DrivePathArgs { vm, drive, path } = parse_args(args)?;
    Ok(run(
        backend,
        vm,
        Action::InsertDriveWireDisk { drive, path },
        "Inserted.",
    ))
}

pub(super) fn dispatch_eject_drivewire_disk(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    let DriveArgs { vm, drive } = parse_args(args)?;
    Ok(run(
        backend,
        vm,
        Action::EjectDriveWireDisk { drive },
        "Ejected.",
    ))
}

#[cfg(test)]
#[path = "media_test.rs"]
mod tests;
