//! LOAD side: [`CocoApp::load_state_from`], resolving a decoded payload's
//! media references into sources [`snapshot::restore`] can consume, and
//! re-syncing every piece of frontend state a snapshot can't carry on its
//! own once the restored machine is swapped in.

use std::path::Path;

use coco_core::cart::Cart;
use coco_core::drivewire::{self, DriveWireImage};
use coco_core::fdc;
use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources, RestoredMachine};
use coco_core::vhd::{self, VHDImage};
use eframe::egui;

use crate::{CocoApp, MPI_SLOT_COUNT, MPISlot, MPIState, RS232Endpoint, ROMSource, host_dw_clock, host_time_source, machine_label};

use super::media_ref::{
    direct_port_rom_path, is_rom_db_pseudo_path, mpi_slot_from_cart, open_if_present,
    read_if_present, resolve_cart_roms, resolve_system_rom,
};

impl CocoApp {
    /// Load, resolve media, and restore a `.ccstate` file, replacing
    /// [`CocoApp::machine`] wholesale and re-syncing every piece of
    /// frontend state a snapshot can't carry on its own — see
    /// [`Self::apply_restored_machine`]. Version/magic/media errors surface
    /// verbatim (by design, per `docs/plan-save-states.md` — they're already
    /// user-showable). Deliberately does NOT touch the window title: the
    /// caller's `egui::Context` may belong to a different viewport than the
    /// machine's own window (the manager's resume path runs on the MANAGER
    /// window's context), so direct-boot call sites reissue the title
    /// themselves via [`Self::refresh_window_title`].
    pub(crate) fn load_state_from(&mut self, path: &Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let payload = snapshot::load(&bytes).map_err(|e| e.to_string())?;
        let media = payload.media.clone();
        let (sources, mut notes) = self.resolve_media_sources(&payload.media)?;
        let restored = snapshot::restore(payload, sources).map_err(|e| e.to_string())?;
        self.apply_restored_machine(restored, &media, &mut notes);

        let mut toast = "State loaded".to_string();
        if !notes.is_empty() {
            toast.push_str(": ");
            toast.push_str(&notes.join("; "));
        }
        self.set_toast(toast);
        Ok(())
    }

    /// Reissue the machine window's title for the current variant — the
    /// restored machine may be a different variant than whatever ran before
    /// a load (reissuing when the variant didn't change is cheap and
    /// idempotent). Split out of [`Self::load_state_from`] because
    /// `ctx.send_viewport_cmd` targets the context's *current* viewport:
    /// the manager's resume path runs on the manager window's root context
    /// and must never retitle it, while every direct-boot call site
    /// (`boot.rs`'s `--state`, the Machine menu's Load State/quick-load) IS
    /// the machine's own window and calls this right after a successful
    /// load.
    pub(crate) fn refresh_window_title(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "cocovm — {}",
            machine_label(self.machine.config.variant)
        )));
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
        let mut vhds: [Option<VHDImage>; vhd::DRIVE_COUNT] = Default::default();
        for (i, mr) in media.vhds.iter().enumerate().take(vhd::DRIVE_COUNT) {
            if let Some(mr) = mr {
                vhds[i] = open_if_present(mr, "VHD", &mut warnings).map(VHDImage::File);
            }
        }
        let mut drivewire: [Option<DriveWireImage>; drivewire::DRIVE_COUNT] = Default::default();
        for (i, mr) in media.drivewire.iter().enumerate().take(drivewire::DRIVE_COUNT) {
            if let Some(mr) = mr {
                drivewire[i] = open_if_present(mr, "DriveWire image", &mut warnings).map(DriveWireImage::File);
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
    /// dropped by (de)serialization ([`coco_core::rtc::DistoRTC`]'s time
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
                .filter(|n| !matches!(n, snapshot::RestoreNote::RTCPlaceholderTime))
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
    /// alike — `Cart::as_*` searches both): [`coco_core::rtc::DistoRTC`]'s
    /// time source, the DriveWire clock, and the Deluxe RS-232 endpoint
    /// (always restored as loopback — see [`snapshot::RestoreNote::RS232EndpointLoopback`]).
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
            Some(mr) if is_rom_db_pseudo_path(&mr.path) => self.rom_source = ROMSource::ComposedCoco12,
            Some(mr) => self.rom_source = ROMSource::File(mr.path.clone()),
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
                        Cart::ROMPak(_) | Cart::BankedROMPak(_) | Cart::GamesMasterCartridge(_) | Cart::Orch90(_) => {
                            new_cart_path = direct_port_rom_path(media);
                        }
                        Cart::DistoRTC(_) => new_rtc_direct = true,
                        Cart::DeluxeRS232(_) => {
                            new_rs232 = Some(RS232Endpoint::Loopback);
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
