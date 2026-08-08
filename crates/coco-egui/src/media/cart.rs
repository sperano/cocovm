//! Cartridges in the port and in MultiPak slots: ROM paks, the Games
//! Master, the Orchestra-90, the Deluxe RS-232 and Sound/Speech paks, the
//! Disto RTC, and the MultiPak itself.

use crate::*;

impl CocoApp {
    /// Load a ROM pak from `path` and insert it, using the current
    /// `autostart_cart` setting. Resets the machine on success (cartridge
    /// insertion is a machine-off operation on real hardware); on failure,
    /// leaves the running cartridge (if any) untouched and records the error
    /// for [`Self::cart_error`] to display.
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
                self.flush_dirty_disks();
                self.machine.insert_cartridge(pak);
                self.machine.power_cycle();
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

    /// Load a Games Master Cartridge image (banked ROM + SN76489A) from
    /// `path` and insert it. Mirrors [`Self::insert_cartridge`]'s ROMPak
    /// path exactly, including the `autostart_cart` choice — GMC games are
    /// autostart game paks (CART* tied to Q), but the checkbox stays
    /// authoritative like it is for plain paks.
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
                self.flush_dirty_disks();
                self.machine.insert_cartridge(cart);
                self.machine.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Load an Orchestra-90/CC ROM from `path` and insert it. Mirrors
    /// [`Self::insert_cartridge`]'s ROMPak path exactly, but there is no
    /// `autostart_cart` choice to honor — [`Orch90::cart_line_ties_q`] always
    /// autostarts, like the real pak's CART*-tied-to-Q wiring.
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
                self.flush_dirty_disks();
                self.machine.insert_cartridge(cart);
                self.machine.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Eject the current cartridge and power-cycle the machine (cartridge
    /// swaps are machine-off operations on real hardware).
    pub(crate) fn eject_cartridge(&mut self) {
        self.flush_dirty_disks();
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None; // whatever was plugged into the port (MPI or not) is gone
        self.rs232 = None;
        self.rs232_eprom_path = None;
        self.rtc_direct = false;
    }

    /// Insert a Deluxe RS-232 Program Pak into the cartridge slot
    /// (cold-restart gated, like plain cartridge insertion). Starts on the
    /// inert loopback endpoint; pick TCP/PTY from the pak's submenu. If a
    /// pak EPROM dump is present at `roms/rs232.rom` it is installed in the
    /// CTS window; the pak is fully usable ROM-less otherwise (OS-9 drivers
    /// and `PEEK`/`POKE` code drive the ACIA registers directly).
    pub(crate) fn insert_rs232(&mut self) {
        self.flush_dirty_disks();
        let mut pak = coco_core::rs232::DeluxeRS232::new();
        let rom_path = rs232_eprom_default_path();
        let eprom_path = if let Ok(bytes) = std::fs::read(&rom_path) {
            pak.set_eprom(&bytes);
            Some(rom_path)
        } else {
            None
        };
        self.machine.insert_cartridge(pak);
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None;
        self.rs232 = Some(RS232Endpoint::Loopback);
        self.rs232_eprom_path = eprom_path;
    }

    /// Wire the inserted RS-232 pak to a freshly bound endpoint of `kind`
    /// (the menu's Loopback/TCP/PTY selection). Binding failures (port in
    /// use, pty exhaustion) land in [`Self::cart_error`] and leave the
    /// current endpoint in place.
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
                        // Show the address actually bound, so ":0" (OS-assigned
                        // port) displays usably.
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

    /// Plug the Sound/Speech Cartridge into the cartridge slot (cold-restart
    /// gated, like every other direct-port cartridge swap). No file to load
    /// and no autostart concept — unlike [`Self::insert_cartridge`]'s ROM
    /// paks, this can't fail.
    pub(crate) fn insert_ssc(&mut self) {
        self.flush_dirty_disks();
        self.machine.insert_cartridge(SoundSpeechCartridge::new());
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None; // plugging straight into the port removes any MPI
        self.rs232 = None;
        self.rs232_eprom_path = None;
    }

    /// Insert a Multi-Pak Interface into the cartridge slot (cold-restart
    /// gated, like plain cartridge insertion): swaps out whatever was
    /// plugged directly into the port for an empty 4-slot MPI with its
    /// front-panel switch on slot 4 ([`DEFAULT_MPI_SWITCH_SLOT`]).
    pub(crate) fn insert_multipak(&mut self) {
        self.flush_dirty_disks();
        self.machine
            .insert_cartridge(MultiPak::new(DEFAULT_MPI_SWITCH_SLOT));
        self.machine.power_cycle();
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

    /// Remove the Multi-Pak Interface — and everything plugged into it —
    /// restoring the plain empty cartridge slot.
    pub(crate) fn eject_multipak(&mut self) {
        self.flush_dirty_disks();
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.mpi = None;
        self.cart_path = None;
        self.disk_paths = [None, None];
    }

    /// Load a ROM pak into MPI `slot` (0-3), using the current
    /// `autostart_cart` setting. Mirrors [`Self::insert_cartridge`] but
    /// targets one slot of the already-inserted MPI instead of the whole
    /// cartridge port.
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
                self.flush_dirty_disks();
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, pak);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::ROMPak(path);
                }
                self.machine.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Load a Games Master Cartridge image into MPI `slot`. Mirrors
    /// [`Self::mpi_insert_rompak`] — see [`Self::insert_gmc`].
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
                self.flush_dirty_disks();
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, cart);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::GamesMasterCartridge(path);
                }
                self.machine.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Load an Orchestra-90/CC ROM into MPI `slot`. Mirrors
    /// [`Self::mpi_insert_rompak`], minus the `autostart_cart` choice — see
    /// [`Self::insert_orch90`].
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
                self.flush_dirty_disks();
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, cart);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::Orch90(path);
                }
                self.machine.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Insert the FD-502 disk controller into MPI `slot`, unless one is
    /// already installed in a different slot (the FD-502 latch only ever
    /// models one controller). Mirrors [`Self::ensure_disk_controller`]'s
    /// cold-start rationale, but targets one MPI slot instead of the whole
    /// cartridge port.
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
        self.flush_dirty_disks();
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, DiskCart::new(rom.into_boxed_slice()));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::FD502;
        }
        self.disk_paths = [None, None];
        self.machine.power_cycle();
    }

    /// Insert the Sound/Speech Cartridge into MPI `slot`. Mirrors
    /// [`Self::insert_ssc`] but targets one MPI slot instead of the whole
    /// cartridge port — any number of slots can each hold one (unlike the
    /// FD-502's single-controller restriction).
    pub(crate) fn mpi_insert_ssc(&mut self, slot: usize) {
        self.flush_dirty_disks();
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, SoundSpeechCartridge::new());
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::SoundSpeechCartridge;
        }
        self.machine.power_cycle();
    }

    /// Eject whatever is plugged into MPI `slot`, restoring its empty slot.
    pub(crate) fn mpi_eject_slot(&mut self, slot: usize) {
        let was_fd502 = matches!(
            self.mpi.as_ref().map(|m| &m.slots[slot]),
            Some(MPISlot::FD502)
        );
        if was_fd502 {
            self.flush_dirty_disks();
            self.disk_paths = [None, None];
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.eject(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::Empty;
        }
        self.machine.power_cycle();
    }

    /// Move the MPI's front-panel switch to `slot`. A running program's own
    /// write to `$FF7F` overrides the switch until the next reset
    /// ([`coco_core::cart::MultiPak::set_switch`]).
    pub(crate) fn mpi_set_switch(&mut self, slot: usize) {
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.set_switch(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.switch = slot;
        }
    }

    /// Plug a Disto RTC directly into the cartridge port, running on the
    /// host's local clock (cold-restart gated like any cartridge swap). The
    /// RTC has no boot ROM, so this pairs with a VHD boot (NitrOS-9 `emudsk`)
    /// rather than the FD-502 — for RTC + floppies, use a Multi-Pak slot.
    pub(crate) fn insert_rtc(&mut self) {
        self.flush_dirty_disks();
        self.machine
            .insert_cartridge(DistoRTC::new(host_time_source()));
        self.machine.power_cycle();
        self.rtc_direct = true;
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None;
    }

    /// Eject a directly-plugged Disto RTC, restoring the empty port.
    pub(crate) fn eject_rtc(&mut self) {
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.rtc_direct = false;
    }

    /// Insert a Disto RTC into MPI `slot` (0-3). Mirrors
    /// [`Self::mpi_insert_fd502`]; only one RTC is allowed across the
    /// machine, since two would shadow each other at `$FF50`.
    pub(crate) fn mpi_insert_rtc(&mut self, slot: usize) {
        if self.machine.bus.cart.as_disto_rtc().is_some() {
            self.cart_error = Some("A Disto RTC is already installed in another slot.".to_string());
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, DistoRTC::new(host_time_source()));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::DistoRTC;
        }
        self.machine.power_cycle();
    }
}
