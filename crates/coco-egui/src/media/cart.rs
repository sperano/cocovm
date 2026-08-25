//! Cartridges in the port and in MultiPak slots: ROM paks, the Games
//! Master, the Orchestra-90, the Deluxe RS-232 and Sound/Speech paks, the
//! Disto RTC, and the MultiPak itself.

use crate::*;

impl CocoApp {
    /// Loads a ROM pak from `path` and inserts it, resetting the machine on success
    /// (cartridge swaps are machine-off ops). Aborts on a failed
    /// dirty-floppy flush, leaving state untouched; error in
    /// [`Self::cart_error`].
    pub(crate) fn insert_cartridge(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match ROMPak::from_bytes(&bytes, self.autostart_cart) {
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
    /// it. Honors `autostart_cart`, even though GMC's own CART* line ties to Q by default.
    pub(crate) fn insert_gmc(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match GamesMasterCartridge::from_bytes(&bytes, self.autostart_cart) {
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

    /// Loads an Orchestra-90/CC ROM from `path` and inserts it. No `autostart_cart` choice
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

    /// Ejects the current cartridge and power-cycles the machine. Aborts, leaving
    /// the cartridge in place, if a dirty floppy it would destroy fails to write back.
    pub(crate) fn eject_cartridge(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine.eject_cartridge();
        self.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None; // whatever was plugged into the port (MPI or not) is gone
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.rtc_direct = false;
    }

    /// Inserts a Deluxe RS-232 Program Pak, starting on the inert loopback endpoint (pick
    /// TCP/PTY from its submenu). Installs the EPROM dump at
    /// `roms/rs232.rom` if present; fully usable ROM-less otherwise.
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
            RS232EndpointKind::Loopback => {
                pak.set_endpoint(Box::new(coco_core::serial::Loopback::new()));
                self.rs232 = Some(RS232Endpoint::Loopback);
            }
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

    /// Plugs the Sound/Speech Cartridge into the cartridge slot. No file to load, so
    /// unlike [`Self::insert_cartridge`] this can't fail — but shares its
    /// abort-on-failed-flush contract.
    pub(crate) fn insert_ssc(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine.insert_cartridge(SoundSpeechCartridge::new());
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

    /// Removes the Multi-Pak Interface and everything plugged into it, restoring the empty
    /// slot. Aborts, leaving the MPI in place, if a dirty floppy any of
    /// its slots holds fails to flush.
    pub(crate) fn eject_multipak(&mut self) {
        if !self.flush_dirty_disks_or_report() {
            return;
        }
        self.machine.eject_cartridge();
        self.power_cycle();
        self.mpi = None;
        self.cart_path = None;
        self.disk_paths = [None, None];
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

    /// Loads a ROM pak into MPI `slot` (0-3). Only a dirty floppy in `slot` itself (i.e.
    /// `slot` holds the FD-502) can abort this — see [`Self::mpi_flush_before_replacing_slot`].
    pub(crate) fn mpi_insert_rompak(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match ROMPak::from_bytes(&bytes, self.autostart_cart) {
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
    pub(crate) fn mpi_insert_gmc(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match GamesMasterCartridge::from_bytes(&bytes, self.autostart_cart) {
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
    /// [`Self::mpi_insert_rompak`]'s flush contract, minus the `autostart_cart` choice.
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
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, SoundSpeechCartridge::new());
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::SoundSpeechCartridge;
        }
        self.power_cycle();
    }

    /// Ejects whatever is plugged into MPI `slot`. Aborts, leaving contents in place, if
    /// `slot` holds the FD-502 and a dirty floppy fails to write back.
    pub(crate) fn mpi_eject_slot(&mut self, slot: usize) {
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.eject(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::Empty;
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

    /// Eject a directly-plugged Disto RTC, restoring the empty port.
    pub(crate) fn eject_rtc(&mut self) {
        self.machine.eject_cartridge();
        self.power_cycle();
        self.rtc_direct = false;
    }

    /// Inserts a Disto RTC into MPI `slot` (0-3). Only one is allowed across the machine —
    /// two would shadow each other at `$FF50`.
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
}

#[cfg(test)]
#[path = "cart_test.rs"]
mod tests;
