//! Frontend save-state UX (`docs/plan-save-states.md` "Frontend UX"): the
//! Machine-menu Save/Load State + Quick Save/Load slots, their keyboard
//! chords, the `--state` CLI flag (`main.rs`'s `Cli`), and the status-bar
//! toast — all built on top of the engine in
//! [`coco_core::snapshot`], which this module is the only caller of.
//!
//! [`CocoApp::save_state_to`]/[`CocoApp::load_state_from`] are the two
//! entry points; everything else here is either UI chrome around them or the
//! fiddly frontend-side re-injection [`coco_core::snapshot::restore`] can't
//! do itself (host-only resources, path mirrors, pacing — see
//! [`CocoApp::apply_restored_machine`]).

use std::path::{Path, PathBuf};

use coco_core::cart::Cart;
use coco_core::drivewire::{self, DwImage};
use coco_core::snapshot::{
    self, MediaCheck, MediaRef, MediaRefs, MediaSources, RestoredMachine, SlotRomRef,
};
use coco_core::vhd::{self, VhdImage};
use coco_core::fdc;
use eframe::egui;

use crate::{
    Coco12RomResult, CocoApp, MPI_SLOT_COUNT, MPISlot, MPIState, ROM_DB_PSEUDO_PATH_PREFIX,
    RomSource, Rs232Endpoint, compose_coco12_rom, dev_roms_dir, disk_basic_rom_path,
    host_dw_clock, host_time_source, machine_label, paths, rom_db_pseudo_path,
};

/// Number of quick-save/quick-load slots the Machine menu exposes.
pub(crate) const QUICK_SLOTS: usize = 3;

/// Subdirectory of [`paths::data_dir`] holding quick-save slot files
/// (`<dir>/slot-<n>.ccstate`, 1-based).
const SAVE_STATES_SUBDIR: &str = "save-states";

/// How long a status-bar toast stays visible after [`CocoApp::set_toast`].
pub(crate) const TOAST_SECS: f64 = 4.0;

/// Physical keys `slot` (0-based) binds to: `Num1`/`Num2`/`Num3` for the
/// three [`QUICK_SLOTS`].
const QUICK_SLOT_KEYS: [egui::Key; QUICK_SLOTS] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3];

/// COMMAND+SHIFT+`<n>` quick-saves state slot `slot` — the SHIFTed sibling
/// of [`load_slot_shortcut`]'s COMMAND+`<n>`. Checked against every existing
/// binding (`kbd_help.rs`'s bare F-keys, `new_vm::NEW_MACHINE_SHORTCUT` =
/// ⌘N): free on every platform `egui::Modifiers::COMMAND` targets.
pub(crate) fn save_slot_shortcut(slot: usize) -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        QUICK_SLOT_KEYS[slot],
    )
}

/// COMMAND+`<n>` quick-loads state slot `slot`.
pub(crate) fn load_slot_shortcut(slot: usize) -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, QUICK_SLOT_KEYS[slot])
}

/// One line for the keyboard-help window (`kbd_help.rs`) naming every
/// quick-slot chord, formatted per-platform (⌘ on macOS, Ctrl elsewhere) via
/// [`egui::Context::format_shortcut`].
pub(crate) fn slot_shortcuts_hint(ctx: &egui::Context) -> String {
    let loads: Vec<String> =
        (0..QUICK_SLOTS).map(|s| ctx.format_shortcut(&load_slot_shortcut(s))).collect();
    let saves: Vec<String> =
        (0..QUICK_SLOTS).map(|s| ctx.format_shortcut(&save_slot_shortcut(s))).collect();
    format!(
        "{}: quick-load state slot 1/2/3   ·   {}: quick-save",
        loads.join(" / "),
        saves.join(" / ")
    )
}

/// `<data_dir>/save-states/slot-<n>.ccstate` (1-based) for `slot` (0-based).
/// `None` when [`paths::data_dir`] can't determine a home directory.
fn quick_slot_path(slot: usize) -> Option<PathBuf> {
    let dir = paths::data_dir()?.join(SAVE_STATES_SUBDIR);
    Some(dir.join(format!("slot-{}.ccstate", slot + 1)))
}

/// Menu-item label for `slot`: its last-modified time if the slot file
/// exists, else "(empty)".
fn quick_slot_label(slot: usize) -> String {
    let n = slot + 1;
    let mtime = quick_slot_path(slot)
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    match mtime {
        Some(t) => format!("Slot {n} ({})", format_mtime(t)),
        None => format!("Slot {n} (empty)"),
    }
}

fn format_mtime(t: std::time::SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = t.into();
    local.format("%Y-%m-%d %H:%M").to_string()
}

impl CocoApp {
    /// Show `msg` in the status bar for [`TOAST_SECS`] seconds.
    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    /// The current toast text, if one is still within [`TOAST_SECS`] of
    /// [`Self::set_toast`] — clears itself once expired. Called once per
    /// frame from the status bar (`main.rs`'s `draw_chrome`).
    pub(crate) fn toast_message(&mut self) -> Option<String> {
        let (msg, at) = self.toast.as_ref()?;
        if at.elapsed().as_secs_f64() > TOAST_SECS {
            self.toast = None;
            return None;
        }
        Some(msg.clone())
    }

    /// The Machine menu's Save/Load State section, drawn right after Reset
    /// (`main.rs`'s `draw_chrome`): file-dialog Save/Load plus the
    /// [`QUICK_SLOTS`] Quick Save/Quick Load submenus.
    pub(crate) fn draw_save_state_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Save State…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CoCo save state", &["ccstate"])
                .set_file_name("state.ccstate")
                .save_file()
                && let Err(e) = self.save_state_to(&path)
            {
                self.cart_error = Some(e);
            }
        }
        if ui.button("Load State…").clicked() {
            ui.close();
            if let Some(path) =
                rfd::FileDialog::new().add_filter("CoCo save state", &["ccstate"]).pick_file()
                && let Err(e) = self.load_state_from(&path, ui.ctx())
            {
                self.cart_error = Some(e);
            }
        }
        ui.menu_button("Quick Save", |ui| {
            for slot in 0..QUICK_SLOTS {
                let button = egui::Button::new(quick_slot_label(slot))
                    .shortcut_text(ui.ctx().format_shortcut(&save_slot_shortcut(slot)));
                if ui.add(button).clicked() {
                    self.quick_save(slot);
                    ui.close();
                }
            }
        });
        ui.menu_button("Quick Load", |ui| {
            for slot in 0..QUICK_SLOTS {
                let occupied = quick_slot_path(slot).is_some_and(|p| p.is_file());
                let button = egui::Button::new(quick_slot_label(slot))
                    .shortcut_text(ui.ctx().format_shortcut(&load_slot_shortcut(slot)));
                if ui.add_enabled(occupied, button).clicked() {
                    self.quick_load(slot, ui.ctx());
                    ui.close();
                }
            }
        });
    }

    /// Quick Save `slot`: like "Save State…" but to a fixed per-slot path
    /// under [`paths::data_dir`] instead of an `rfd` dialog, creating
    /// [`SAVE_STATES_SUBDIR`] on demand. Failures land in
    /// [`CocoApp::cart_error`], like every other menu action.
    pub(crate) fn quick_save(&mut self, slot: usize) {
        let Some(path) = quick_slot_path(slot) else {
            self.cart_error = Some("no data directory found for quick-save slots".to_string());
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            self.cart_error = Some(format!("could not create {}: {e}", dir.display()));
            return;
        }
        if let Err(e) = self.save_state_to(&path) {
            self.cart_error = Some(e);
        }
    }

    /// Quick Load `slot` — the load-side sibling of [`Self::quick_save`].
    pub(crate) fn quick_load(&mut self, slot: usize, ctx: &egui::Context) {
        let Some(path) = quick_slot_path(slot) else {
            self.cart_error = Some("no data directory found for quick-save slots".to_string());
            return;
        };
        if let Err(e) = self.load_state_from(&path, ctx) {
            self.cart_error = Some(e);
        }
    }

    /// Flush dirty media first (so path+hash refs describe the on-disk
    /// truth — [`snapshot::save`]'s own contract), build [`MediaRefs`] from
    /// the paths this app already tracks, and write the encoded `.ccstate`
    /// bytes to `path`.
    pub(crate) fn save_state_to(&mut self, path: &Path) -> Result<(), String> {
        self.flush_media();
        let media = self.build_media_refs()?;
        let bytes = snapshot::save(&self.machine, &media).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        self.set_toast("State saved".to_string());
        Ok(())
    }

    /// Load, resolve media, and restore a `.ccstate` file, replacing
    /// [`CocoApp::machine`] wholesale and re-syncing every piece of
    /// frontend state a snapshot can't carry on its own — see
    /// [`Self::apply_restored_machine`]. Version/magic/media errors surface
    /// verbatim (by design, per `docs/plan-save-states.md` — they're already
    /// user-showable). Takes `ctx` (every call site already has one — the
    /// menu/quick-slot paths through a `ui`, the CLI `--state` path through
    /// `eframe`'s `CreationContext`) so a cross-variant load can update the
    /// window title, mirroring [`CocoApp::create_vm`]'s own.
    pub(crate) fn load_state_from(&mut self, path: &Path, ctx: &egui::Context) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let payload = snapshot::load(&bytes).map_err(|e| e.to_string())?;
        let media = payload.media.clone();
        let (sources, mut notes) = self.resolve_media_sources(&payload.media)?;
        let restored = snapshot::restore(payload, sources).map_err(|e| e.to_string())?;
        self.apply_restored_machine(restored, &media, &mut notes);

        // The restored machine may be a different variant than whatever was
        // running before the load (`create_vm`'s own title update is the
        // fresh-boot sibling of this one) — always reissue it, even when the
        // variant didn't change, since that's cheap and idempotent.
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "coco-rs — {}",
            machine_label(self.machine.config.variant)
        )));

        let mut toast = "State loaded".to_string();
        if !notes.is_empty() {
            toast.push_str(": ");
            toast.push_str(&notes.join("; "));
        }
        self.set_toast(toast);
        Ok(())
    }

    /// Build this app's [`MediaRefs`] from the paths it already tracks
    /// (`cart_path`/`mpi`/`rs232_eprom_path`/`disk_paths`/`vhd_paths`/
    /// `dw_paths`/`tape_path`), hashing each referenced FILE — pak images
    /// are mirror-filled in memory, so in-memory bytes never equal file
    /// bytes, hence always [`snapshot::sha256_file`] on the source.
    fn build_media_refs(&mut self) -> Result<MediaRefs, String> {
        let system_rom = Some(self.system_rom_media_ref());
        let cart_roms = self.collect_cart_roms()?;

        let mut disks: Vec<Option<MediaRef>> = vec![None; fdc::DRIVE_COUNT];
        for (i, path) in self.disk_paths.iter().enumerate() {
            if let Some(path) = path {
                disks[i] = Some(hash_media_ref(path)?);
            }
        }
        let mut vhds: Vec<Option<MediaRef>> = vec![None; vhd::DRIVE_COUNT];
        for (i, path) in self.vhd_paths.iter().enumerate() {
            if let Some(path) = path {
                vhds[i] = Some(hash_media_ref(path)?);
            }
        }
        let mut drivewire: Vec<Option<MediaRef>> = vec![None; drivewire::DRIVE_COUNT];
        for (i, path) in self.dw_paths.iter().enumerate() {
            if let Some(path) = path {
                drivewire[i] = Some(hash_media_ref(path)?);
            }
        }
        let tape = match &self.tape_path {
            Some(path) => Some(hash_media_ref(path)?),
            None => None,
        };

        Ok(MediaRefs { system_rom, cart_roms, disks, vhds, drivewire, tape })
    }

    /// [`MediaRefs::system_rom`]: the frontend resolved/composed the boot
    /// ROM at boot time ([`RomSource`]) — a real file records its path but
    /// hashes the boot-time bytes live from `self.machine.bus.rom` rather
    /// than re-reading the file (a ROM file modified mid-session should
    /// produce a mismatch WARNING on restore, not a silently-passing wrong
    /// hash computed from bytes the running machine never actually used); a
    /// CoCo 1/2 `rom_db`-composed image has no single file, so this records
    /// a pseudo-path ([`rom_db_pseudo_path`]) and hashes the composed bytes
    /// the same way — this arm already worked this way before this
    /// function's `File` arm was brought in line with it.
    fn system_rom_media_ref(&self) -> MediaRef {
        let path = match &self.rom_source {
            RomSource::File(path) => path.clone(),
            RomSource::ComposedCoco12 => rom_db_pseudo_path(self.machine.config.variant),
        };
        MediaRef { path, sha256: snapshot::sha256_hex(&self.machine.bus.rom) }
    }

    /// [`MediaRefs::cart_roms`]: one entry per ROM-bearing cart the app
    /// tracks a path for — `cart_path`/`mpi` (direct port / MultiPak slots)
    /// plus the FD-502's Disk BASIC ROM ([`disk_basic_rom_path`]) and the
    /// Deluxe RS-232 pak's optional EPROM (`rs232_eprom_path`, only present
    /// when one was actually mounted — the pak also runs ROM-less).
    fn collect_cart_roms(&mut self) -> Result<Vec<SlotRomRef>, String> {
        let mut out = Vec::new();
        if let Some(mpi) = &self.mpi {
            // Snapshot the slot paths first — this ends the immutable
            // borrow of `self.mpi` before the hashing loop below needs
            // `self` again for `hash_media_ref`'s error path.
            let paths: Vec<(Option<u8>, PathBuf)> = mpi
                .slots
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| {
                    let path = match slot {
                        MPISlot::ROMPak(p) | MPISlot::Gmc(p) | MPISlot::Orch90(p) => p.clone(),
                        MPISlot::FD502 => disk_basic_rom_path(),
                        MPISlot::Empty | MPISlot::DistoRTC | MPISlot::Ssc => return None,
                    };
                    Some((Some(i as u8), path))
                })
                .collect();
            for (mpi_slot, path) in paths {
                out.push(SlotRomRef { mpi_slot, rom: hash_media_ref(&path)? });
            }
        } else {
            if let Some(path) = &self.cart_path {
                out.push(SlotRomRef { mpi_slot: None, rom: hash_media_ref(path)? });
            } else if self.machine.bus.cart.as_disk_cart().is_some() {
                out.push(SlotRomRef {
                    mpi_slot: None,
                    rom: hash_media_ref(&disk_basic_rom_path())?,
                });
            }
            if let Some(path) = &self.rs232_eprom_path {
                out.push(SlotRomRef { mpi_slot: None, rom: hash_media_ref(path)? });
            }
        }
        Ok(out)
    }

    /// Resolve a decoded payload's [`MediaRefs`] into [`MediaSources`] for
    /// [`snapshot::restore`], plus warnings for every hash mismatch found
    /// along the way (per `docs/plan-save-states.md`: "load with warning").
    /// A referenced file that's simply missing/unreadable is left absent
    /// from the returned [`MediaSources`] rather than erroring here —
    /// [`snapshot::restore`] itself collects every still-missing reference
    /// (matched against the deserialized cart tree, so it can name each
    /// one's exact role) into one batched, already user-showable error; this
    /// function only needs to say what's *wrong*, not what's *absent*. The
    /// one exception is a mounted `.wav` tape that fails to decode
    /// ([`Self::resolve_tape`]): that's neither "wrong" (a warning) nor
    /// "absent" (silently becomes `MissingMedia` downstream, burying the
    /// actual reason) — it's a hard error straight out of this function.
    fn resolve_media_sources(&self, media: &MediaRefs) -> Result<(MediaSources, Vec<String>), String> {
        let mut warnings = Vec::new();
        let system_rom = resolve_system_rom(media, &mut warnings);
        let cart_roms = resolve_cart_roms(media, &mut warnings);

        // Every loop below caps at this build's own `DRIVE_COUNT` even though
        // `media.*` is a `Vec` that a hand-edited (or genuinely newer-schema)
        // payload could make longer: `MediaSources`' arrays are fixed at
        // that size (`docs/plan-save-states.md` phase-5 review — indexing
        // past it would panic), and any entry beyond it is separately
        // rejected with a proper [`snapshot::SnapshotError::InvalidPayload`]
        // by [`snapshot::restore`] right after this function returns, so
        // silently not resolving it here costs nothing.
        let mut disks: [Option<Vec<u8>>; fdc::DRIVE_COUNT] = Default::default();
        for (i, mr) in media.disks.iter().enumerate().take(fdc::DRIVE_COUNT) {
            if let Some(mr) = mr {
                disks[i] = read_if_present(mr, "floppy", &mut warnings);
            }
        }
        let mut vhds: [Option<VhdImage>; vhd::DRIVE_COUNT] = Default::default();
        for (i, mr) in media.vhds.iter().enumerate().take(vhd::DRIVE_COUNT) {
            if let Some(mr) = mr {
                vhds[i] = open_if_present(mr, "VHD", &mut warnings).map(VhdImage::File);
            }
        }
        let mut drivewire: [Option<DwImage>; drivewire::DRIVE_COUNT] = Default::default();
        for (i, mr) in media.drivewire.iter().enumerate().take(drivewire::DRIVE_COUNT) {
            if let Some(mr) = mr {
                drivewire[i] = open_if_present(mr, "DriveWire image", &mut warnings).map(DwImage::File);
            }
        }
        let tape = match &media.tape {
            Some(mr) => self.resolve_tape(mr, &mut warnings)?,
            None => None,
        };

        Ok((MediaSources { system_rom, cart_roms, disks, vhds, drivewire, tape }, warnings))
    }

    /// [`MediaSources::tape`]: raw bytes, or — sniffed by the `RIFF` magic,
    /// same as [`CocoApp::insert_tape`] — demodulated through
    /// [`coco_core::cassette_wav::decode_wav`] for a tape that was mounted
    /// from (and never re-saved as) a `.wav` recording. A decode failure is
    /// a hard `Err` (not a `warnings` entry): leaving it as `None` here
    /// would surface downstream as [`snapshot::SnapshotError::MissingMedia`]
    /// from [`snapshot::restore`], which the caller (`Self::load_state_from`)
    /// turns straight into its own error string — the specific "couldn't
    /// decode this .wav" reason would never reach the user, only "tape:
    /// (no reference recorded)"-style boilerplate.
    fn resolve_tape(&self, mr: &MediaRef, warnings: &mut Vec<String>) -> Result<Option<Vec<u8>>, String> {
        let Some(raw) = read_if_present(mr, "tape", warnings) else { return Ok(None) };
        if raw.starts_with(b"RIFF") {
            coco_core::cassette_wav::decode_wav(&raw, self.machine.cpu_hz()).map(Some).map_err(|e| {
                format!("tape ({}) could not be decoded as .wav: {e}", mr.path.display())
            })
        } else {
            Ok(Some(raw))
        }
    }

    /// Swap in a freshly-restored machine and re-sync every piece of
    /// frontend state a snapshot can't carry itself: host-only resources
    /// dropped by (de)serialization ([`coco_core::rtc::DistoRtc`]'s time
    /// source, the DriveWire clock, the RS-232 endpoint — always restored as
    /// loopback), the printer paper window's handle, path mirrors rebuilt
    /// from `media` and the restored cart tree, and emulation pacing.
    /// Appends every [`RestoredMachine::notes`] entry to `notes` for the
    /// caller's toast. `aspect_correct`/other UI prefs are NOT in the
    /// snapshot and are left untouched, same as the running/paused state.
    fn apply_restored_machine(
        &mut self,
        restored: RestoredMachine,
        media: &MediaRefs,
        notes: &mut Vec<String>,
    ) {
        self.machine = restored.machine;

        // A key held during quick-save must not stay held forever after a
        // later quick-load — mirrors `Self::set_mode`'s own call. The
        // SERIALIZED matrix round-trips intact (that's correct: the snapshot
        // itself faithfully records "this key was down"); it's only this
        // frontend, applying a restore to a live session, that chooses to
        // clear it, the same way a fresh keyboard-mode switch does.
        self.machine.bus.keyboard.release_all();

        self.reinject_host_only_resources();

        // Now that the RTC's host time source is back (just above),
        // `RestoreNote::RtcPlaceholderTime` no longer describes this
        // session's state — it's only true for a caller that DOESN'T
        // immediately re-sync the clock the way this one just did (a
        // headless tool, a test) — so drop it before converting the rest of
        // the notes to toast text.
        notes.extend(
            restored
                .notes
                .into_iter()
                .filter(|n| !matches!(n, snapshot::RestoreNote::RtcPlaceholderTime))
                .map(|n| n.to_string()),
        );

        // Printer: re-link the paper window if the restored sink came back
        // as a live DMP-105; a file capture always restores as stopped
        // (`restored.notes` already says so when it applies) — either way
        // the frontend no longer owns a live capture file handle.
        match self.machine.bus.bitbanger.dmp105_handle() {
            Some(handle) => self.paper_window.resync(Some(handle)),
            None => self.paper_window.detach(),
        }
        self.print_capture_path = None;

        self.update_rom_source_from_media(media);
        self.rebuild_cart_mirrors(media);
        self.rebuild_media_path_mirrors(media);

        // Pacing/audio: drop any time owed to the wall clock so resuming
        // doesn't "catch up" across the load, like a pause
        // (`Self::step_emulation`'s own doc). `Machine.audio_buffer` is
        // `#[serde(skip)]`, rebuilt empty by `Machine::after_restore`
        // (already run inside `snapshot::restore`), so there's no stale
        // backlog on the core side to drop here.
        self.last_update = None;
        self.field_debt = 0.0;
        self.type_ahead.clear();
    }

    /// Re-inject every host-only resource `#[serde(skip)]` dropped by the
    /// round trip, wherever the owning device landed (port or MPI slot
    /// alike — `Cart::as_*` searches both): [`coco_core::rtc::DistoRtc`]'s
    /// time source, the DriveWire clock, and the Deluxe RS-232 endpoint
    /// (always restored as loopback — see [`snapshot::RestoreNote::Rs232EndpointLoopback`]).
    fn reinject_host_only_resources(&mut self) {
        if let Some(rtc) = self.machine.bus.cart.as_disto_rtc() {
            rtc.set_time_source(host_time_source());
        }
        if let Some(dw) = self.machine.bus.drivewire.as_mut() {
            dw.set_clock(host_dw_clock());
        }
        if let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() {
            pak.set_endpoint(Box::new(coco_core::serial::Loopback::new()));
        }
    }

    /// `rom_source` must follow the restored snapshot's own ROM ref, not
    /// stay pointed at whatever the pre-load session booted with — a later
    /// save (after a cross-variant load) would otherwise record the WRONG
    /// `rom_source` for the tree it's actually saving, producing a snapshot
    /// that restores garbage yet still passes its own hash check. `None`
    /// (no ref recorded at all) is the one case that keeps the current
    /// value: a hand-built payload with no system-ROM reference is the only
    /// way to reach it, and there's no better answer than "whatever the
    /// resolver already supplied".
    fn update_rom_source_from_media(&mut self, media: &MediaRefs) {
        match &media.system_rom {
            Some(mr) if is_rom_db_pseudo_path(&mr.path) => self.rom_source = RomSource::ComposedCoco12,
            Some(mr) => self.rom_source = RomSource::File(mr.path.clone()),
            None => {}
        }
    }

    /// Rebuild `disk_paths`/`vhd_paths`/`dw_paths`/`tape_path` from `media`.
    /// `.get(i)`, not `media.disks[i]`: `media.*` is a `Vec` (a future
    /// `DRIVE_COUNT` change mustn't brick old snapshots — see `MediaRefs`'s
    /// doc comment), possibly shorter than this app's own fixed-size
    /// path-mirror arrays, e.g. a snapshot that never mounted any disk at
    /// all.
    fn rebuild_media_path_mirrors(&mut self, media: &MediaRefs) {
        for (i, path) in self.disk_paths.iter_mut().enumerate() {
            *path = media.disks.get(i).and_then(Option::as_ref).map(|r| r.path.clone());
        }
        for (i, path) in self.vhd_paths.iter_mut().enumerate() {
            *path = media.vhds.get(i).and_then(Option::as_ref).map(|r| r.path.clone());
        }
        for (i, path) in self.dw_paths.iter_mut().enumerate() {
            *path = media.drivewire.get(i).and_then(Option::as_ref).map(|r| r.path.clone());
        }
        self.tape_path = media.tape.as_ref().map(|r| r.path.clone());
    }

    /// Rebuild `cart_path`/`mpi`/`rtc_direct`/`rs232`/`rs232_eprom_path`
    /// from the just-restored cart tree ([`Cart::slots_mut`]) plus `media`
    /// (for the paths — `slots_mut`'s `&mut Cart`s only say what *kind* of
    /// cart is where, not the file it came from).
    fn rebuild_cart_mirrors(&mut self, media: &MediaRefs) {
        self.cart_path = None;
        self.rtc_direct = false;
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.mpi = None;

        let switch = self.machine.bus.cart.as_multipak().map(|mp| mp.switch_slot());
        match switch {
            Some(switch) => {
                let mut slots: [MPISlot; MPI_SLOT_COUNT] = std::array::from_fn(|_| MPISlot::Empty);
                for (mpi_slot, cart) in self.machine.bus.cart.slots_mut() {
                    if let Some(i) = mpi_slot {
                        slots[i as usize] = mpi_slot_from_cart(cart, i, media);
                    }
                }
                self.mpi = Some(MPIState { switch, slots });
            }
            None => {
                let mut new_cart_path = None;
                let mut new_rtc_direct = false;
                let mut new_rs232 = None;
                let mut new_rs232_eprom_path = None;
                for (_, cart) in self.machine.bus.cart.slots_mut() {
                    match cart {
                        Cart::RomPak(_) | Cart::BankedRomPak(_) | Cart::Gmc(_) | Cart::Orch90(_) => {
                            new_cart_path = direct_port_rom_path(media);
                        }
                        Cart::DistoRtc(_) => new_rtc_direct = true,
                        Cart::DeluxeRs232(_) => {
                            new_rs232 = Some(Rs232Endpoint::Loopback);
                            new_rs232_eprom_path = direct_port_rom_path(media);
                        }
                        _ => {}
                    }
                }
                self.cart_path = new_cart_path;
                self.rtc_direct = new_rtc_direct;
                self.rs232 = new_rs232;
                self.rs232_eprom_path = new_rs232_eprom_path;
            }
        }
    }
}

/// The recorded `mpi_slot: None` cart-ROM path, if any — shared by
/// [`CocoApp::rebuild_cart_mirrors`]'s direct-port `cart_path` and
/// `rs232_eprom_path` cases.
fn direct_port_rom_path(media: &MediaRefs) -> Option<PathBuf> {
    media.cart_roms.iter().find(|r| r.mpi_slot.is_none()).map(|r| r.rom.path.clone())
}

/// The [`MPISlot`] `cart` (at slot `i`) should display as, using `media` for
/// the display path of every ROM-bearing variant.
fn mpi_slot_from_cart(cart: &Cart, i: u8, media: &MediaRefs) -> MPISlot {
    let rom_path =
        || media.cart_roms.iter().find(|r| r.mpi_slot == Some(i)).map(|r| r.rom.path.clone());
    match cart {
        Cart::RomPak(_) | Cart::BankedRomPak(_) => rom_path().map(MPISlot::ROMPak).unwrap_or(MPISlot::Empty),
        Cart::Gmc(_) => rom_path().map(MPISlot::Gmc).unwrap_or(MPISlot::Empty),
        Cart::Orch90(_) => rom_path().map(MPISlot::Orch90).unwrap_or(MPISlot::Empty),
        Cart::DiskCart(_) => MPISlot::FD502,
        Cart::DistoRtc(_) => MPISlot::DistoRTC,
        Cart::Ssc(_) => MPISlot::Ssc,
        _ => MPISlot::Empty,
    }
}

/// Hash the file at `path` into a [`MediaRef`] (SAVE side).
fn hash_media_ref(path: &Path) -> Result<MediaRef, String> {
    let sha256 = snapshot::sha256_file(path)
        .map_err(|e| format!("could not hash {}: {e}", path.display()))?;
    Ok(MediaRef { path: path.to_path_buf(), sha256 })
}

/// LOAD side: `mr`'s bytes if its file is present (pushing a `warnings`
/// entry first when the hash no longer matches), or `None` if it's missing
/// — see [`CocoApp::resolve_media_sources`]'s doc for why a missing file
/// isn't an error here.
fn read_if_present(mr: &MediaRef, role: &str, warnings: &mut Vec<String>) -> Option<Vec<u8>> {
    match mr.verify() {
        MediaCheck::Missing => None,
        MediaCheck::Mismatch { .. } => {
            warnings.push(mismatch_warning(role, mr));
            std::fs::read(&mr.path).ok()
        }
        MediaCheck::Ok => std::fs::read(&mr.path).ok(),
    }
}

/// [`read_if_present`]'s sibling for VHD/DriveWire sources, which need a
/// read-write file handle rather than bytes.
fn open_if_present(mr: &MediaRef, role: &str, warnings: &mut Vec<String>) -> Option<std::fs::File> {
    match mr.verify() {
        MediaCheck::Missing => None,
        MediaCheck::Mismatch { .. } => {
            warnings.push(mismatch_warning(role, mr));
            open_read_write(&mr.path).ok()
        }
        MediaCheck::Ok => open_read_write(&mr.path).ok(),
    }
}

/// Mirrors [`CocoApp::insert_vhd`]/[`CocoApp::insert_dw_disk`]'s own
/// `OpenOptions` — VHD/DriveWire writes hit the backing file directly.
fn open_read_write(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

fn mismatch_warning(role: &str, mr: &MediaRef) -> String {
    format!(
        "{role} ({}) doesn't match the hash recorded in this save state; loaded anyway",
        mr.path.display()
    )
}

/// True if `path` is one of [`RomSource::ComposedCoco12`]'s pseudo-paths
/// ([`ROM_DB_PSEUDO_PATH_PREFIX`], built by [`rom_db_pseudo_path`]) rather
/// than a real filesystem path.
fn is_rom_db_pseudo_path(path: &Path) -> bool {
    path.to_str().is_some_and(|s| s.starts_with(ROM_DB_PSEUDO_PATH_PREFIX))
}

/// [`MediaSources::system_rom`]: a real path reads and verifies like any
/// other reference; a [`ROM_DB_PSEUDO_PATH_PREFIX`] pseudo-path recomposes
/// the CoCo 1/2 flat image from [`dev_roms_dir`] and hash-compares (never
/// "missing" purely because the pseudo-path itself isn't a real file — only
/// when no local Color BASIC dump exists to compose from at all).
fn resolve_system_rom(media: &MediaRefs, warnings: &mut Vec<String>) -> Option<Box<[u8]>> {
    let mr = media.system_rom.as_ref()?;
    if is_rom_db_pseudo_path(&mr.path) {
        return match compose_coco12_rom(&dev_roms_dir()) {
            Coco12RomResult::Composed { image, .. } => {
                let actual = snapshot::sha256_hex(&image);
                if actual != mr.sha256 {
                    warnings.push(format!(
                        "system ROM ({}): local Color/Extended BASIC dumps differ from the ones \
                         this state was saved with; loaded anyway",
                        mr.path.display()
                    ));
                }
                Some(image)
            }
            Coco12RomResult::NoColorBasic => None,
        };
    }
    read_if_present(mr, "system ROM", warnings).map(Vec::into_boxed_slice)
}

fn resolve_cart_roms(media: &MediaRefs, warnings: &mut Vec<String>) -> Vec<(Option<u8>, Vec<u8>)> {
    media
        .cart_roms
        .iter()
        .filter_map(|slot_ref| {
            read_if_present(&slot_ref.rom, "cartridge ROM", warnings)
                .map(|bytes| (slot_ref.mpi_slot, bytes))
        })
        .collect()
}

#[cfg(test)]
#[path = "save_state_test.rs"]
mod tests;
