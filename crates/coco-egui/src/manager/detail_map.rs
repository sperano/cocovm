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
/// the inverse of [`ManagerApp::pack_def`].
pub(super) fn seed_form(def: &machine_def::MachineDef) -> new_vm::MachineForm {
    let mut form = new_vm::MachineForm::new("detail");
    form.config = def
        .to_machine_config()
        .expect("list entries are validated on load/save");
    let media = &def.media;
    (
        form.cartridge,
        form.mpi_slots,
        form.mpi_switch,
        form.rs232_endpoint,
    ) = new_vm::seed_peripherals(&def.peripherals);
    let media_choice = |raw: &Option<String>| match raw {
        Some(s) => new_vm::MediaChoice::File(PathBuf::from(s)),
        None => new_vm::MediaChoice::None,
    };
    form.disks = [media_choice(&media.disk0), media_choice(&media.disk1)];
    form.tape = media_choice(&media.tape);
    form.vhds = [media_choice(&media.vhd0), media_choice(&media.vhd1)];
    form.drivewire = def.drivewire.clone();
    form.display = def.display();
    form.tv = crate::display::TVSettings {
        scanline_pct: def.ui.tv_scanline,
        noise_pct: def.ui.tv_noise,
        overscan_pct: def.ui.tv_overscan,
    }
    .clamped();
    form.serial = def.ports.serial.into();
    // Indexed by `coco_core::joystick::{RIGHT, LEFT}`, like `new_vm::MachineForm::joy_sources`.
    form.joy_sources[coco_core::joystick::RIGHT] = def.ui.joy_right.into();
    form.joy_sources[coco_core::joystick::LEFT] = def.ui.joy_left.into();
    form.hires[coco_core::joystick::RIGHT] = def.ui.hires_right.into();
    form.hires[coco_core::joystick::LEFT] = def.ui.hires_left.into();
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
        def.peripherals = new_vm::pack_peripherals(
            &form.cartridge,
            &form.mpi_slots,
            form.mpi_switch,
            &form.rs232_endpoint,
        );
        def.drivewire = form.drivewire.clone();
        def.validate_drivewire()?;
        self.pack_media(slug, form, &mut def)?;
        pack_ui(form, &mut def);
        Ok(def)
    }

    /// Resolve the disk/tape/VHD picks through [`Self::record_media_choice`]
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

/// Pack the form's remaining `[ports]`/`[ui]` picks: the Serial sink,
/// TV settings, per-port joystick source and hi-res interface, and keyboard
/// mode.
fn pack_ui(form: &new_vm::MachineForm, def: &mut machine_def::MachineDef) {
    def.ports.serial = form.serial.into();
    def.ui.tv_scanline = form.tv.scanline_pct;
    def.ui.tv_noise = form.tv.noise_pct;
    def.ui.tv_overscan = form.tv.overscan_pct;
    def.ui.joy_right = form.joy_sources[coco_core::joystick::RIGHT].into();
    def.ui.joy_left = form.joy_sources[coco_core::joystick::LEFT].into();
    def.ui.hires_right = form.hires[coco_core::joystick::RIGHT].into();
    def.ui.hires_left = form.hires[coco_core::joystick::LEFT].into();
    def.ui.kb_mode = match form.kb_mode {
        crate::KbMode::Positional => machine_def::KbModeDTO::Positional,
        crate::KbMode::Symbolic => machine_def::KbModeDTO::Symbolic,
    };
}
