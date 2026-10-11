//! The media actions beyond floppies — tape, cartridges and the MultiPak
//! switch, VHDs, and DriveWire disks — each through the same `CocoApp`
//! method the VM window or the launch path uses. Like `insert_disk`, they
//! change the running session only, never the machine definition.

use std::path::PathBuf;

use coco_core::MachineVariant;

use crate::control::{CartridgeTarget, Reply};
use crate::machine_def::{self, CartridgeDTO, DosRom, SlotDTO};
use crate::{CocoApp, launch};

use super::ManagerApp;
use super::dispatch::{check_drive, with_cart_error};

/// Appended when a new cartridge failed after the old one came out.
const OLD_CARTRIDGE_REMOVED: &str =
    "The previous cartridge was removed; call list_vms to see what is installed now.";
const NO_MULTIPAK: &str =
    "no MultiPak is in the cartridge port; insert one with insert_cartridge, kind \"mpi\"";
const NO_TAPE: &str = "no tape is mounted; call insert_tape first";

impl ManagerApp {
    /// `insert_tape`: [`CocoApp::insert_tape`], after saving the old tape
    /// the way it does ([`write_back_tape`]).
    pub(super) fn insert_tape_action(
        &mut self,
        vm: &Option<String>,
        path: PathBuf,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.vm_mut(idx);
        let wav_error = write_back_tape(app)?;
        with_cart_error(app, |app| app.insert_tape(path))?;
        wav_outcome(wav_error, "inserted")
    }

    /// `eject_tape`: [`CocoApp::eject_tape`], after saving the tape the way
    /// it does ([`write_back_tape`]).
    pub(super) fn eject_tape_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.vm_mut(idx);
        let wav_error = write_back_tape(app)?;
        with_cart_error(app, CocoApp::eject_tape)?;
        wav_outcome(wav_error, "ejected")
    }

    /// `rewind_tape`: the Tape menu's Rewind Tape.
    pub(super) fn rewind_tape_action(&mut self, vm: &Option<String>) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.vm_mut(idx);
        if app.tape_path.is_none() {
            return Err(NO_TAPE.to_string());
        }
        app.machine.bus.cassette.rewind();
        Ok(Reply::Done)
    }

    /// `insert_cartridge`: empty the port or slot, then mount `target` the
    /// way launching a definition naming it would
    /// ([`launch::mount_cartridge`], [`launch::mount_slot`]).
    pub(super) fn insert_cartridge_action(
        &mut self,
        vm: &Option<String>,
        target: CartridgeTarget,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let slug = self.entries[idx].slug.clone();
        let app = self.vm_mut(idx);
        match target {
            CartridgeTarget::Port(cartridge) => {
                check_fits(app, &port_needs(&cartridge))?;
                with_cart_error(app, CocoApp::eject_cartridge)?;
                with_cart_error(app, |app| launch::mount_cartridge(app, &cartridge, &slug))
            }
            CartridgeTarget::Slot { slot, occupant } => {
                require_multipak(app)?;
                check_fits(app, &slot_needs(&occupant))?;
                with_cart_error(app, |app| app.mpi_eject_slot(slot))?;
                with_cart_error(app, |app| launch::mount_slot(app, slot, &occupant, &slug))
            }
        }
        .map_err(|error| format!("{error}\n{OLD_CARTRIDGE_REMOVED}"))
    }

    /// `remove_cartridge`: empty the port ([`CocoApp::eject_cartridge`]) or
    /// one MultiPak slot ([`CocoApp::mpi_eject_slot`]).
    pub(super) fn remove_cartridge_action(
        &mut self,
        vm: &Option<String>,
        slot: Option<usize>,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.vm_mut(idx);
        match slot {
            None => with_cart_error(app, CocoApp::eject_cartridge),
            Some(slot) => {
                require_multipak(app)?;
                with_cart_error(app, |app| app.mpi_eject_slot(slot))
            }
        }
    }

    /// `set_mpi_switch`: [`CocoApp::mpi_set_switch`].
    pub(super) fn set_mpi_switch_action(
        &mut self,
        vm: &Option<String>,
        slot: usize,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        let app = self.vm_mut(idx);
        require_multipak(app)?;
        app.mpi_set_switch(slot);
        Ok(Reply::Done)
    }

    /// `insert_vhd`: [`CocoApp::insert_vhd`], in one of the drives the UI
    /// exposes, like `insert_disk`.
    pub(super) fn insert_vhd_action(
        &mut self,
        vm: &Option<String>,
        drive: usize,
        path: PathBuf,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        check_drive(drive)?;
        with_cart_error(self.vm_mut(idx), |app| app.insert_vhd(drive, path))
    }

    /// `eject_vhd`: [`CocoApp::eject_vhd`].
    pub(super) fn eject_vhd_action(
        &mut self,
        vm: &Option<String>,
        drive: usize,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        check_drive(drive)?;
        self.vm_mut(idx).eject_vhd(drive);
        Ok(Reply::Done)
    }

    /// `insert_drivewire_disk` (`Some`) and `eject_drivewire_disk` (`None`):
    /// [`CocoApp::set_session_dw_disk`], which shares its mount and cross-VM
    /// lease with the settings path (`CocoApp::apply_drivewire_settings`).
    pub(super) fn set_drivewire_disk_action(
        &mut self,
        vm: &Option<String>,
        drive: usize,
        path: Option<PathBuf>,
    ) -> Result<Reply, String> {
        let idx = self.resolve_vm(vm, true)?;
        check_drivewire_drive(drive)?;
        let slug = self.entries[idx].slug.clone();
        let app = self.vm_mut(idx);
        if app.machine.bus.drivewire.is_none() {
            return Err(format!(
                "DriveWire is off for VM '{slug}'; turn it on in the machine's DriveWire settings"
            ));
        }
        app.set_session_dw_disk(drive, path)?;
        Ok(Reply::Done)
    }
}

/// Save the mounted tape's recording, as [`CocoApp::insert_tape`] and
/// [`CocoApp::eject_tape`] do before they change tapes. `Err` when the
/// `.cas` save failed, which makes them change nothing; `Ok(Some)` when only
/// the optional `.wav` copy failed, which they report but go past.
fn write_back_tape(app: &mut CocoApp) -> Result<Option<String>, String> {
    match app.write_back_tape() {
        Ok(()) => Ok(None),
        Err(error) if app.machine.bus.cassette.dirty() => Err(error),
        Err(error) => Ok(Some(error)),
    }
}

/// A tape change that went through, still reported as an error when the old
/// recording's `.wav` copy failed.
fn wav_outcome(wav_error: Option<String>, done: &str) -> Result<Reply, String> {
    match wav_error {
        None => Ok(Reply::Done),
        Some(error) => Err(format!(
            "{error}\nThe tape was {done}; only the .wav copy of the previous recording failed."
        )),
    }
}

/// [`check_drive`]'s twin for DriveWire's larger drive count.
fn check_drivewire_drive(drive: usize) -> Result<(), String> {
    if drive >= coco_core::drivewire::DRIVE_COUNT {
        return Err(format!(
            "drive must be 0..{}",
            coco_core::drivewire::DRIVE_COUNT
        ));
    }
    Ok(())
}

fn require_multipak(app: &CocoApp) -> Result<(), String> {
    if app.mpi.is_none() {
        return Err(NO_MULTIPAK.to_string());
    }
    Ok(())
}

/// What a cartridge needs from the machine it goes into.
struct Needs {
    games_master: bool,
    hdbdos: bool,
}

fn port_needs(cartridge: &CartridgeDTO) -> Needs {
    Needs {
        games_master: cartridge.contains_games_master(),
        hdbdos: cartridge.uses_hdbdos(),
    }
}

fn slot_needs(occupant: &SlotDTO) -> Needs {
    Needs {
        games_master: matches!(occupant, SlotDTO::GamesMaster { .. }),
        hdbdos: matches!(
            occupant,
            SlotDTO::FD502 {
                dos_rom: DosRom::HdbDosDw3
            }
        ),
    }
}

/// The checks a definition's cartridge passes before launch
/// (`MachineDef::validate_drivewire`), against the running machine.
fn check_fits(app: &CocoApp, needs: &Needs) -> Result<(), String> {
    if needs.hdbdos && app.machine.config.variant != MachineVariant::Coco3 {
        return Err(machine_def::HDBDOS_NEEDS_COCO3.to_string());
    }
    if needs.games_master && app.machine.bus.drivewire.is_some() {
        return Err(machine_def::DRIVEWIRE_GMC_CONFLICT.to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "media_test.rs"]
mod tests;
