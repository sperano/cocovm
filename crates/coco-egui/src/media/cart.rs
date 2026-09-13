//! Cartridges in the port and in MultiPak slots: ROM paks, the Games
//! Master, the Orchestra-90, the Deluxe RS-232 and Sound/Speech paks, the
//! Disto RTC, and the MultiPak itself.

use crate::*;

/// A Sound/Speech Cartridge built around its TMS7040 firmware
/// ([`rom_load::ssc_firmware_rom_path`]) and SP0256-AL2 ROM
/// ([`rom_load::sp0256_rom_path`]); the error names the file that is
/// missing or the wrong size.
pub(crate) fn sound_speech_cartridge() -> Result<SoundSpeechCartridge, String> {
    sound_speech_cartridge_in(&installed_roms_dir())
}

/// [`sound_speech_cartridge`] reading both images from `roms_dir`.
fn sound_speech_cartridge_in(roms_dir: &Path) -> Result<SoundSpeechCartridge, String> {
    let firmware_path = roms_dir.join(rom_load::SSC_FIRMWARE_ROM);
    let speech_path = roms_dir.join(rom_load::SP0256_ROM);
    let firmware = std::fs::read(&firmware_path).map_err(|e| {
        format!(
            "could not read TMS7040 firmware {}: {e}",
            firmware_path.display()
        )
    })?;
    let speech = std::fs::read(&speech_path).map_err(|e| {
        format!(
            "could not read SP0256-AL2 ROM {}: {e}",
            speech_path.display()
        )
    })?;
    SoundSpeechCartridge::new(&firmware, &speech).map_err(|e| {
        format!(
            "{}: {e}",
            match e {
                coco_core::ssc::SSCROMError::Firmware(_) => firmware_path.display(),
                coco_core::ssc::SSCROMError::Speech(_) => speech_path.display(),
            }
        )
    })
}

impl CocoApp {
    /// Loads a ROM pak from `path` and inserts it, resetting the machine on success
    /// (cartridge swaps are machine-off ops). `autostart` ties CART* to Q so
    /// the pak runs at power-up. Aborts on a failed dirty-floppy flush, leaves
    /// state untouched, and reports the error in [`Self::cart_error`].
    pub(crate) fn insert_cartridge(&mut self, path: PathBuf, autostart: bool) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match ROMPak::from_bytes(&bytes, autostart) {
            Ok(pak) => {
                if !self.flush_dirty_disks_or_report() {
                    return;
                }
                self.machine.insert_cartridge(pak);
                self.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
                self.rs232 = None; // ...and any RS-232 pak
                self.rs232_eprom_path = None;
                self.rtc_direct = false; // ... and any directly-plugged RTC
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Loads a Games Master Cartridge image (banked ROM + SN76489A) from `path` and inserts
    /// it. Honors `autostart`, even though GMC's own CART* line ties to Q by default.
    pub(crate) fn insert_gmc(&mut self, path: PathBuf, autostart: bool) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match GamesMasterCartridge::from_bytes(&bytes, autostart) {
            Ok(cart) => {
                if !self.flush_dirty_disks_or_report() {
                    return;
                }
                self.machine.insert_cartridge(cart);
                self.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Loads an Orchestra-90/CC ROM from `path` and inserts it. No `autostart` choice
    /// to honor — [`Orch90::cart_line_ties_q`] always autostarts, like the
    /// real pak's CART*-tied-to-Q wiring.
    ///
    /// [`Orch90::cart_line_ties_q`]: coco_core::cart::Cartridge::cart_line_ties_q
    pub(crate) fn insert_orch90(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match Orch90::from_rom_bytes(&bytes) {
            Ok(cart) => {
                if !self.flush_dirty_disks_or_report() {
                    return;
                }
                self.machine.insert_cartridge(cart);
                self.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Inserts a Deluxe RS-232 Program Pak, starting on the inert loopback endpoint
    /// (`launch::mount_rs232` rebinds it to the definition's configured endpoint
    /// afterward). Installs the EPROM dump at `roms/rs232.rom` if present; fully usable
    /// ROM-less otherwise.
    pub(crate) fn insert_rs232(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        let mut pak = coco_core::rs232::DeluxeRS232::new();
        let rom_path = rs232_eprom_default_path();
        let eprom_path = if let Ok(bytes) = std::fs::read(&rom_path) {
            pak.set_eprom(&bytes);
            Some(rom_path)
        } else {
            None
        };
        self.machine.insert_cartridge(pak);
        self.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None;
        self.rs232 = Some(RS232Endpoint::Loopback);
        self.rs232_eprom_path = eprom_path;
    }

    /// Wires the inserted RS-232 pak to a freshly bound endpoint of `kind`. Binding
    /// failures land in [`Self::cart_error`], leaving the current endpoint in place.
    pub(crate) fn rs232_set_endpoint(&mut self, kind: RS232EndpointKind) {
        let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() else {
            return;
        };
        match kind {
            RS232EndpointKind::TCP => {
                match coco_core::serial::TCPEndpoint::bind(&self.rs232_tcp_addr) {
                    Ok(ep) => {
                        // Show the actually-bound address so ":0"
                        // (OS-assigned port) displays usably.
                        let addr = ep
                            .local_addr()
                            .map_or_else(|_| self.rs232_tcp_addr.clone(), |a| a.to_string());
                        pak.set_endpoint(Box::new(ep));
                        self.rs232 = Some(RS232Endpoint::TCP(addr));
                    }
                    Err(e) => {
                        self.cart_error =
                            Some(format!("could not listen on {}: {e}", self.rs232_tcp_addr));
                    }
                }
            }
            #[cfg(unix)]
            RS232EndpointKind::PTY => match coco_core::serial::PTYEndpoint::new() {
                Ok(ep) => {
                    let path = ep.path().to_string();
                    pak.set_endpoint(Box::new(ep));
                    self.rs232 = Some(RS232Endpoint::PTY(path));
                }
                Err(e) => {
                    self.cart_error = Some(format!("could not open a pty: {e}"));
                }
            },
        }
    }

    /// Plugs the Sound/Speech Cartridge into the cartridge slot. No file to
    /// pick, but its SP0256-AL2 ROM must be installed; a missing one lands in
    /// [`Self::cart_error`] like any other unreadable cartridge ROM. Shares
    /// [`Self::insert_cartridge`]'s abort-on-failed-flush contract.
    pub(crate) fn insert_ssc(&mut self) {
        let ssc = match sound_speech_cartridge() {
            Ok(ssc) => ssc,
            Err(e) => {
                self.cart_error = Some(e);
                return;
            }
        };
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine.insert_cartridge(ssc);
        self.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None; // plugging straight into the port removes any MPI
        self.rs232 = None;
        self.rs232_eprom_path = None;
    }

    /// Inserts a Multi-Pak Interface, swapping out whatever was plugged directly into the
    /// port for an empty 4-slot MPI with its switch on slot 4 ([`DEFAULT_MPI_SWITCH_SLOT`]).
    pub(crate) fn insert_multipak(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine
            .insert_cartridge(MultiPak::new(DEFAULT_MPI_SWITCH_SLOT));
        self.power_cycle();
        self.mpi = Some(MPIState {
            switch: DEFAULT_MPI_SWITCH_SLOT,
            slots: std::array::from_fn(|_| MPISlot::Empty),
        });
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.rtc_direct = false;
    }

    /// Flushes before replacing MPI `slot`'s contents, but only when it holds the FD-502 —
    /// other cartridge kinds can't hold a dirty floppy. `false` means the
    /// caller must abort without mutating anything.
    #[must_use]
    fn mpi_flush_before_replacing_slot(&mut self, slot: usize) -> bool {
        let holds_fd502 = self
            .mpi
            .as_ref()
            .is_some_and(|m| matches!(m.slots[slot], MPISlot::FD502));
        if !holds_fd502 {
            return true;
        }
        if !self.flush_dirty_disks_or_report() {
            return false;
        }
        self.disk_paths = [None, None];
        true
    }

    /// Loads a ROM pak into MPI `slot` (0-3). `autostart` ties CART* to Q so the
    /// pak runs at power-up. Only a dirty floppy in `slot` itself (that is,
    /// `slot` holds the FD-502) can abort this — see [`Self::mpi_flush_before_replacing_slot`].
    pub(crate) fn mpi_insert_rompak(&mut self, slot: usize, path: PathBuf, autostart: bool) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match ROMPak::from_bytes(&bytes, autostart) {
            Ok(pak) => {
                if !self.mpi_flush_before_replacing_slot(slot) {
                    return;
                }
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, pak);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::ROMPak(path);
                }
                self.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Loads a Games Master Cartridge image into MPI `slot`. Mirrors
    /// [`Self::mpi_insert_rompak`]'s target-slot-only flush contract.
    pub(crate) fn mpi_insert_gmc(&mut self, slot: usize, path: PathBuf, autostart: bool) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match GamesMasterCartridge::from_bytes(&bytes, autostart) {
            Ok(cart) => {
                if !self.mpi_flush_before_replacing_slot(slot) {
                    return;
                }
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, cart);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::GamesMasterCartridge(path);
                }
                self.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Loads an Orchestra-90/CC ROM into MPI `slot`. Mirrors
    /// [`Self::mpi_insert_rompak`]'s flush contract, minus the `autostart` choice.
    pub(crate) fn mpi_insert_orch90(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match Orch90::from_rom_bytes(&bytes) {
            Ok(cart) => {
                if !self.mpi_flush_before_replacing_slot(slot) {
                    return;
                }
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, cart);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::Orch90(path);
                }
                self.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Inserts the FD-502 disk controller into MPI `slot`, unless one is already installed
    /// in a different slot — the FD-502 latch only ever models one controller.
    pub(crate) fn mpi_insert_fd502(&mut self, slot: usize) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.cart_error = Some("An FD-502 is already installed in another slot.".to_string());
            return;
        }
        let path = disk_basic_rom_path();
        let rom = match std::fs::read(&path) {
            Ok(rom) => rom,
            Err(e) => {
                self.cart_error = Some(format!(
                    "could not read Disk BASIC ROM {}: {e}",
                    path.display()
                ));
                return;
            }
        };
        // No flush needed: `as_disk_cart` already refused unless no disk cart exists anywhere.
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, DiskCart::new(rom.into_boxed_slice()));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::FD502;
        }
        self.disk_paths = [None, None];
        self.power_cycle();
    }

    /// Inserts the Sound/Speech Cartridge into MPI `slot`. Unlike the FD-502, any number
    /// of slots can each hold one.
    pub(crate) fn mpi_insert_ssc(&mut self, slot: usize) {
        let ssc = match sound_speech_cartridge() {
            Ok(ssc) => ssc,
            Err(e) => {
                self.cart_error = Some(e);
                return;
            }
        };
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, ssc);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::SoundSpeechCartridge;
        }
        self.power_cycle();
    }

    /// Moves the MPI's front-panel switch to `slot`. A running program's own write to
    /// `$FF7F` overrides it until the next reset.
    pub(crate) fn mpi_set_switch(&mut self, slot: usize) {
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.set_switch(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.switch = slot;
        }
    }

    /// Plugs a Disto RTC directly into the cartridge port, running on the host's local
    /// clock. Has no boot ROM, so pairs with a VHD boot rather than the
    /// FD-502 — for RTC + floppies, use a Multi-Pak slot.
    pub(crate) fn insert_rtc(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine
            .insert_cartridge(DistoRTC::new(host_time_source()));
        self.power_cycle();
        self.rtc_direct = true;
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None;
    }

    /// Inserts a Disto RTC into MPI `slot` (0-3). Only one is allowed across
    /// the machine — two would shadow each other at `$FF50`.
    pub(crate) fn mpi_insert_rtc(&mut self, slot: usize) {
        if self.machine.bus.cart.as_disto_rtc().is_some() {
            self.cart_error = Some("A Disto RTC is already installed in another slot.".to_string());
            return;
        }
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, DistoRTC::new(host_time_source()));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::DistoRTC;
        }
        self.power_cycle();
    }

    /// Inserts a Deluxe RS-232 Pak into MPI `slot`, starting on the inert loopback
    /// endpoint like [`Self::insert_rs232`] (`launch::mount_peripherals`'s MPI arm
    /// rebinds it to the definition's configured endpoint afterward). Only one is
    /// allowed across the machine — two would fight over the shared ACIA at
    /// `$FF68-$FF6B`, reachable from any slot regardless of switch/`$FF7F` selection
    /// (`coco_rs232.cpp`: the pak decodes the full address bus itself).
    pub(crate) fn mpi_insert_rs232(&mut self, slot: usize) {
        if self.machine.bus.cart.as_deluxe_rs232().is_some() {
            self.cart_error =
                Some("A Deluxe RS-232 Pak is already installed in another slot.".to_string());
            return;
        }
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        let mut pak = coco_core::rs232::DeluxeRS232::new();
        let rom_path = rs232_eprom_default_path();
        let eprom_path = if let Ok(bytes) = std::fs::read(&rom_path) {
            pak.set_eprom(&bytes);
            Some(rom_path)
        } else {
            None
        };
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, pak);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::DeluxeRS232(eprom_path);
        }
        self.rs232 = Some(RS232Endpoint::Loopback);
        self.power_cycle();
    }
}

#[cfg(test)]
#[path = "cart_test.rs"]
mod tests;
