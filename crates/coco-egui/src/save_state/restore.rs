//! LOAD side: [`CocoApp::load_state_from`], resolving a decoded payload's
//! media references into sources that [`snapshot::restore`] can consume, and
//! re-syncing every piece of frontend state a snapshot can't carry on its
//! own once the restored machine is swapped in.

use std::path::Path;

use coco_core::cart::Cart;
use coco_core::drivewire::{self, DWImage};
use coco_core::fdc;
use coco_core::snapshot::{self, MediaRef, MediaRefs, MediaSources, RestoredMachine};
use coco_core::vhd::{self, VHDImage};
use eframe::egui;

use crate::{
    CocoApp, MPI_SLOT_COUNT, MPISlot, MPIState, ROMSource, RS232Endpoint, host_dw_clock,
    host_time_source, machine_label,
};

use super::media_ref::{
    direct_port_rom_path, is_rom_db_pseudo_path, mpi_slot_from_cart, open_if_present,
    read_if_present, resolve_cart_roms, resolve_system_rom,
};

impl CocoApp {
    /// Load, resolve media, and restore a `.ccstate` file, replacing
    /// [`CocoApp::machine`] wholesale. Does not touch the window title —
    /// callers use [`Self::refresh_window_title`] for that.
    pub(crate) fn load_state_from(&mut self, path: &Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
        let mut payload = snapshot::load(&bytes).map_err(|e| e.to_string())?;
        let media = payload.media.clone();
        let ssc_slots: Vec<Option<u8>> = payload
            .machine
            .bus
            .cart
            .slots_mut()
            .into_iter()
            .filter(|(_, cart)| matches!(cart, Cart::SoundSpeechCartridge(_)))
            .map(|(slot, _)| slot)
            .collect();
        let (sources, mut notes) = self.resolve_media_sources(&payload.media, &ssc_slots)?;
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

    /// Reissue the machine window's title for the current variant. Must be
    /// called on the machine window's own context, never the manager's root context.
    pub(crate) fn refresh_window_title(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "cocovm — {}",
            machine_label(self.machine.config.variant)
        )));
    }

    /// Resolve a decoded payload's [`MediaRefs`] into [`MediaSources`], plus
    /// hash-mismatch warnings. Missing files are left absent rather than
    /// errored, except a corrupt `.wav` tape ([`Self::resolve_tape`]).
    fn resolve_media_sources(
        &self,
        media: &MediaRefs,
        ssc_slots: &[Option<u8>],
    ) -> Result<(MediaSources, Vec<String>), String> {
        let mut warnings = Vec::new();
        let system_rom = resolve_system_rom(media, &mut warnings);
        let cart_roms = resolve_cart_roms(media, ssc_slots, &mut warnings);

        // Caps at DRIVE_COUNT: media.* may be a longer Vec, but snapshot::restore rejects
        // oversized payloads separately.
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
        let mut drivewire: [Option<DWImage>; drivewire::DRIVE_COUNT] = Default::default();
        for (i, mr) in media
            .drivewire
            .iter()
            .enumerate()
            .take(drivewire::DRIVE_COUNT)
        {
            if let Some(mr) = mr {
                drivewire[i] =
                    open_if_present(mr, "DriveWire image", &mut warnings).map(DWImage::File);
            }
        }
        let tape = match &media.tape {
            Some(mr) => self.resolve_tape(mr, &mut warnings)?,
            None => None,
        };

        Ok((
            MediaSources {
                system_rom,
                cart_roms,
                disks,
                vhds,
                drivewire,
                tape,
            },
            warnings,
        ))
    }

    /// [`MediaSources::tape`]: raw bytes, or WAV-decoded if the file sniffs
    /// as `RIFF`. A decode failure is a hard error so the reason reaches the user.
    fn resolve_tape(
        &self,
        mr: &MediaRef,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Vec<u8>>, String> {
        let Some(raw) = read_if_present(mr, "tape", warnings) else {
            return Ok(None);
        };
        if raw.starts_with(b"RIFF") {
            coco_core::cassette_wav::decode_wav(&raw, self.machine.cpu_hz())
                .map(Some)
                .map_err(|e| {
                    format!(
                        "tape ({}) could not be decoded as .wav: {e}",
                        mr.path.display()
                    )
                })
        } else {
            Ok(Some(raw))
        }
    }

    /// Swap in a freshly-restored machine and re-sync frontend state that a
    /// snapshot can't carry itself (host-only resources, printer window,
    /// path mirrors, pacing). UI prefs like `kb_mode` are left untouched.
    fn apply_restored_machine(
        &mut self,
        restored: RestoredMachine,
        media: &MediaRefs,
        notes: &mut Vec<String>,
    ) {
        self.machine = restored.machine;

        // A key held at quick-save time must not stay stuck held after quick-load.
        self.machine.bus.keyboard.release_all();

        self.reinject_host_only_resources();

        // Re-assert the display preference's monitor path; the snapshot's GIME
        // config may not match it.
        if let Some(monitor) = self.display.to_monitor(self.machine.config.variant) {
            self.machine.bus.gime.monitor = monitor;
        }

        // RTCPlaceholderTime no longer applies once the host time source is re-synced earlier.
        notes.extend(
            restored
                .notes
                .into_iter()
                .filter(|n| !matches!(n, snapshot::RestoreNote::RTCPlaceholderTime))
                .map(|n| n.to_string()),
        );

        // Re-link the paper window only if the restored sink is a live DMP printer;
        // a file capture restores it stopped.
        match self.machine.bus.bitbanger.printer_handle() {
            Some(handle) => self.paper_window.resync(Some(handle)),
            None => self.paper_window.detach(),
        }
        self.print_capture_path = None;

        self.update_rom_source_from_media(media);
        self.rebuild_cart_mirrors(media);
        self.reapply_configured_rs232();
        self.rebuild_media_path_mirrors(media);

        // Drop time owed to the wall clock (like a pause) and reset the frontend's audio ring
        // buffer/filter history.
        self.reset_audio();
        self.last_update = None;
        self.field_debt = 0.0;
        self.type_ahead.clear();
    }

    /// Re-inject every host-only resource `#[serde(skip)]` dropped by the
    /// round trip: RTC time source, DriveWire clock, RS-232 endpoint (restored
    /// as loopback here; [`Self::reapply_configured_rs232`] rebinds it to a
    /// non-default kind afterward if one was configured).
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

    /// `rom_source` must follow the restored snapshot's own ROM ref, not the
    /// pre-load session's — otherwise a later save would silently record the
    /// wrong source and corrupt the snapshot.
    fn update_rom_source_from_media(&mut self, media: &MediaRefs) {
        match &media.system_rom {
            Some(mr) if is_rom_db_pseudo_path(&mr.path) => {
                self.rom_source = ROMSource::ComposedCoco12
            }
            Some(mr) => self.rom_source = ROMSource::File(mr.path.clone()),
            None => {}
        }
    }

    /// Rebuild `disk_paths`/`vhd_paths`/`dw_paths`/`tape_path` from `media`.
    /// Uses `.get(i)`, not indexing — `media.*` may be shorter than these
    /// fixed-size path-mirror arrays.
    fn rebuild_media_path_mirrors(&mut self, media: &MediaRefs) {
        for (i, path) in self.disk_paths.iter_mut().enumerate() {
            *path = media
                .disks
                .get(i)
                .and_then(Option::as_ref)
                .map(|r| r.path.clone());
        }
        for (i, path) in self.vhd_paths.iter_mut().enumerate() {
            *path = media
                .vhds
                .get(i)
                .and_then(Option::as_ref)
                .map(|r| r.path.clone());
        }
        for (i, path) in self.dw_paths.iter_mut().enumerate() {
            *path = media
                .drivewire
                .get(i)
                .and_then(Option::as_ref)
                .map(|r| r.path.clone());
        }
        self.tape_path = media.tape.as_ref().map(|r| r.path.clone());
    }

    /// Rebuild `cart_path`/`mpi`/`rtc_direct`/`rs232`/`rs232_eprom_path` from
    /// the restored cart tree plus `media` (for the paths `slots_mut` doesn't carry).
    fn rebuild_cart_mirrors(&mut self, media: &MediaRefs) {
        self.cart_path = None;
        self.rtc_direct = false;
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.mpi = None;

        let switch = self
            .machine
            .bus
            .cart
            .as_multipak()
            .map(|mp| mp.switch_slot());
        match switch {
            Some(switch) => {
                let mut slots: [MPISlot; MPI_SLOT_COUNT] = std::array::from_fn(|_| MPISlot::Empty);
                for (mpi_slot, cart) in self.machine.bus.cart.slots_mut() {
                    if let Some(i) = mpi_slot {
                        slots[i as usize] = mpi_slot_from_cart(cart, i, media);
                    }
                }
                if let Some(path) = slots.iter().find_map(|s| match s {
                    MPISlot::DeluxeRS232(path) => Some(path),
                    _ => None,
                }) {
                    self.rs232 = Some(RS232Endpoint::Loopback);
                    self.rs232_eprom_path = path.clone();
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
                        Cart::ROMPak(_) | Cart::BankedROMPak(_) | Cart::GamesMasterCartridge(_) => {
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

    /// Rebind the restored Deluxe RS-232 Pak — bare port or an MPI slot,
    /// found either way through [`Cart::as_deluxe_rs232`]'s `MultiPak`
    /// forwarding — to whatever non-loopback endpoint kind `[peripherals]`
    /// configured at launch (`rs232_configured`, set by
    /// `launch::apply_rs232_endpoint`). The old machine's
    /// endpoint (and any bound TCP listener) is already dropped by the time
    /// this runs, so the address is free to rebind. A `None` leaves the
    /// loopback [`Self::rebuild_cart_mirrors`]/[`Self::reinject_host_only_resources`]
    /// already restored untouched.
    fn reapply_configured_rs232(&mut self) {
        if let Some(kind) = self.rs232_configured {
            self.rs232_set_endpoint(kind);
        }
    }
}
