//! The form↔definition mapping half of the detail pane: [`seed_form`] (a
//! definition into a fresh [`new_vm::MachineForm`]) and
//! [`ManagerApp::pack_def`] (a form's picks back into a definition, its
//! inverse), plus the pieces only they need — the blank-media file-name
//! helpers and [`ManagerApp::record_media_choice`]. Split out of
//! `detail.rs` (whose own doc comment covers the drawing/interaction half)
//! once that file grew past the project's ~500-line file ceiling.

use std::fs;
use std::path::PathBuf;

use crate::{machine_def, new_vm};

use super::{ManagerApp, NO_CONFIG_DIR};

/// [`ManagerApp::record_media_choice`]'s auto-placed cassette file name, for
/// `[media].tape`.
const BLANK_TAPE_FILE: &str = "tape.cas";

/// File name of the auto-placed blank image for Disk N = Blank, recorded in
/// `[media].diskN` as a relative path.
fn blank_disk_file(drive: usize) -> String {
    format!("disk{drive}.dsk")
}

/// [`blank_disk_file`]'s VHD sibling, for `[media].vhdN`.
fn blank_vhd_file(drive: usize) -> String {
    format!("hd{drive}.vhd")
}

/// Seed the detail pane's [`new_vm::MachineForm`] from a saved definition —
/// the inverse of [`ManagerApp::pack_def`], reconstructing the cartridge
/// picture the same way `crate::launch_machine` mounts it.
pub(super) fn seed_form(def: &machine_def::MachineDef) -> new_vm::MachineForm {
    let mut form = new_vm::MachineForm::new("detail");
    form.config = def
        .to_machine_config()
        .expect("list entries are validated on load/save");
    let media = &def.media;
    let fd502 = def.peripherals.fd502 || media.disk0.is_some() || media.disk1.is_some();
    let cart = media.cart.as_deref().map(PathBuf::from);
    if def.peripherals.mpi {
        form.cartridge = new_vm::CartridgeChoice::MPI;
        if let Some(path) = cart {
            form.mpi_slots[0] = new_vm::SlotChoice::ROMPak(path);
        }
        if fd502 {
            form.mpi_slots[crate::MPI_SLOT_COUNT - 1] = new_vm::SlotChoice::FD502;
        }
        if def.peripherals.rtc {
            form.mpi_slots[crate::DEFAULT_RTC_SLOT] = new_vm::SlotChoice::RTC;
        }
    } else if let Some(path) = cart {
        form.cartridge = new_vm::CartridgeChoice::ROMPak(path);
    } else if def.peripherals.rtc {
        form.cartridge = new_vm::CartridgeChoice::RTC;
    } else if def.peripherals.rs232 {
        form.cartridge = new_vm::CartridgeChoice::RS232;
    } else if fd502 {
        form.cartridge = new_vm::CartridgeChoice::FD502;
    }
    let media_choice = |raw: &Option<String>| match raw {
        Some(s) => new_vm::MediaChoice::File(PathBuf::from(s)),
        None => new_vm::MediaChoice::None,
    };
    form.disks = [media_choice(&media.disk0), media_choice(&media.disk1)];
    form.tape = media_choice(&media.tape);
    form.vhds = [media_choice(&media.vhd0), media_choice(&media.vhd1)];
    form.display = def.display();
    form.tv = crate::display::TVSettings {
        scanline_pct: def.ui.tv_scanline,
        noise_pct: def.ui.tv_noise,
    }
    .clamped();
    form.aspect_correct = def.ui.aspect_correct;
    form.serial = def.ports.serial.into();
    // Indexed by `coco_core::joystick::{RIGHT, LEFT}`, like `new_vm::MachineForm::joy_sources`.
    form.joy_sources[coco_core::joystick::RIGHT] = def.ui.joy_right.into();
    form.joy_sources[coco_core::joystick::LEFT] = def.ui.joy_left.into();
    form.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => crate::KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => crate::KbMode::Symbolic,
    };
    form
}

impl ManagerApp {
    /// Resolve one of the edit form's media picks to the string recorded in
    /// the definition's `[media]` section, creating a 0-byte backing file in
    /// `slug`'s artifact dir for a Blank pick. Rewrites the pick to
    /// `File(recorded)` afterward so the combo shows the placed file.
    fn record_media_choice(
        &self,
        slug: &str,
        choice: &mut new_vm::MediaChoice,
        auto_file: String,
    ) -> Result<Option<String>, String> {
        let (path, recorded) = match &*choice {
            new_vm::MediaChoice::None => return Ok(None),
            new_vm::MediaChoice::File(path) => return Ok(Some(path.display().to_string())),
            new_vm::MediaChoice::Blank => {
                let Some(root) = self.artifacts_root.clone() else {
                    return Err(NO_CONFIG_DIR.to_string());
                };
                let artifact_dir = root.join(slug);
                fs::create_dir_all(&artifact_dir)
                    .map_err(|e| format!("{}: {e}", artifact_dir.display()))?;
                (artifact_dir.join(&auto_file), auto_file)
            }
        };
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        *choice = new_vm::MediaChoice::File(PathBuf::from(&recorded));
        Ok(Some(recorded))
    }

    /// Pack the edit form back into a definition, starting from `base` so
    /// everything the form doesn't edit passes through untouched. Errors
    /// leave the definition unwritten and land in the pane's error label.
    pub(super) fn pack_def(
        &self,
        base: &machine_def::MachineDef,
        slug: &str,
        form: &mut new_vm::MachineForm,
    ) -> Result<machine_def::MachineDef, String> {
        let mut def = base.clone();
        def.hardware = machine_def::HardwareDTO::from_config(
            &form.config,
            form.display,
            base.hardware.rom.clone(),
        );
        pack_cartridge(form, &mut def)?;
        self.pack_media(slug, form, &mut def)?;
        pack_ui(form, &mut def);
        Ok(def)
    }

    /// Resolve the disk/tape/VHD picks via [`Self::record_media_choice`]
    /// into `def.media` — the write moment for a Blank pick's backing file,
    /// since every change is a save.
    fn pack_media(
        &self,
        slug: &str,
        form: &mut new_vm::MachineForm,
        def: &mut machine_def::MachineDef,
    ) -> Result<(), String> {
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.disks[drive], blank_disk_file(drive))?;
            match drive {
                0 => def.media.disk0 = recorded,
                _ => def.media.disk1 = recorded,
            }
        }
        def.media.tape =
            self.record_media_choice(slug, &mut form.tape, BLANK_TAPE_FILE.to_string())?;
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.vhds[drive], blank_vhd_file(drive))?;
            match drive {
                0 => def.media.vhd0 = recorded,
                _ => def.media.vhd1 = recorded,
            }
        }
        Ok(())
    }
}

/// Pack the cartridge port's peripherals flags and `[media].cart` from the
/// form's Cartridge/MPI-slot picks. The schema has no slot layout: an
/// MPI-slotted FD-502/RTC is recorded as a plain flag, re-seated by `launch_machine`.
fn pack_cartridge(
    form: &new_vm::MachineForm,
    def: &mut machine_def::MachineDef,
) -> Result<(), String> {
    def.peripherals.fd502 = form.drives_available();
    def.peripherals.mpi = form.cartridge == new_vm::CartridgeChoice::MPI;
    def.peripherals.rtc = form.cartridge == new_vm::CartridgeChoice::RTC
        || form.mpi_slots.contains(&new_vm::SlotChoice::RTC);
    def.peripherals.rs232 = form.cartridge == new_vm::CartridgeChoice::RS232;
    // A ROM Pak (port or MPI-slotted) is recorded as [media].cart; the schema holds only one.
    let mut slotted_paks = form.mpi_slots.iter().filter_map(|slot| match slot {
        new_vm::SlotChoice::ROMPak(path) => Some(path),
        _ => None,
    });
    def.media.cart = match &form.cartridge {
        new_vm::CartridgeChoice::ROMPak(path) => Some(path.display().to_string()),
        new_vm::CartridgeChoice::MPI => slotted_paks.next().map(|p| p.display().to_string()),
        _ => None,
    };
    if slotted_paks.next().is_some() {
        return Err(
            "a machine definition records a single ROM Pak — leave at most one slot \
             with a pak"
                .to_string(),
        );
    }
    Ok(())
}

/// Pack the form's remaining `[ports]`/`[ui]` picks: the Serial sink,
/// aspect correction, per-port joystick source, and keyboard mode.
fn pack_ui(form: &new_vm::MachineForm, def: &mut machine_def::MachineDef) {
    def.ports.serial = form.serial.into();
    def.ui.aspect_correct = form.aspect_correct;
    def.ui.tv_scanline = form.tv.scanline_pct;
    def.ui.tv_noise = form.tv.noise_pct;
    def.ui.joy_right = form.joy_sources[coco_core::joystick::RIGHT].into();
    def.ui.joy_left = form.joy_sources[coco_core::joystick::LEFT].into();
    def.ui.kb_mode = match form.kb_mode {
        crate::KbMode::Positional => machine_def::KbModeDTO::Positional,
        crate::KbMode::Symbolic => machine_def::KbModeDTO::Symbolic,
    };
}
