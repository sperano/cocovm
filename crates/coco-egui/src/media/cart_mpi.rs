//! MultiPak Interface slot occupants: loading/ejecting a cartridge into one
//! of the MPI's four slots — split out of `cart.rs` once that file grew past
//! the project's ~500-line ceiling. The MPI itself (`insert_multipak`) and
//! every device that can also plug straight into the bare cartridge port
//! stay in `cart.rs`; this file is only the per-slot half.

use super::cart::{CART_AUTOSTART, orchestra_90, sound_speech_cartridge};
use crate::*;

impl CocoApp {
    /// Flushes before replacing MPI `slot`'s contents, but only when it holds the FD-502 —
    /// other cartridge kinds can't hold a dirty floppy. `false` means the
    /// caller must abort without mutating anything.
    #[must_use]
    pub(super) fn mpi_flush_before_replacing_slot(&mut self, slot: usize) -> bool {
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
        self.disk_rom_path = None;
        true
    }

    /// Loads a ROM pak into MPI `slot` (0-3). Only a dirty floppy in `slot` itself
    /// (that is, `slot` holds the FD-502) can abort this — see
    /// [`Self::mpi_flush_before_replacing_slot`].
    pub(crate) fn mpi_insert_rompak(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match ROMPak::from_bytes(&bytes, CART_AUTOSTART) {
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
        match GamesMasterCartridge::from_bytes(&bytes, CART_AUTOSTART) {
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

    /// Inserts the Orchestra-90/CC into MPI `slot`. Mirrors
    /// [`Self::mpi_insert_ssc`]: no file to pick, and a missing or wrong-size
    /// ROM lands in [`Self::cart_error`].
    pub(crate) fn mpi_insert_orch90(&mut self, slot: usize) {
        let cart = match orchestra_90() {
            Ok(cart) => cart,
            Err(e) => {
                self.cart_error = Some(e);
                return;
            }
        };
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, cart);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::Orch90;
        }
        self.power_cycle();
    }

    /// Inserts the FD-502 disk controller into MPI `slot`, unless one is already installed
    /// in a different slot — the FD-502 latch only ever models one controller.
    pub(crate) fn mpi_insert_fd502(&mut self, slot: usize, dos_rom: machine_def::DosRom) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.cart_error = Some("An FD-502 is already installed in another slot.".to_string());
            return;
        }
        let path = dos_rom_path(dos_rom);
        let rom = match std::fs::read(&path) {
            Ok(rom) => rom,
            Err(e) => {
                self.cart_error = Some(format!("could not read DOS ROM {}: {e}", path.display()));
                return;
            }
        };
        // No flush needed: `as_disk_cart` already refused unless no disk cart exists anywhere.
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, DiskCart::new(rom.into_boxed_slice()));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::FD502;
            self.disk_rom_path = Some(path);
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

    /// Inserts a CoCo Max Hi-Res Input Module into MPI `slot`. See
    /// [`CocoApp::insert_cocomax`]'s CoCo 1/2-only doc.
    pub(crate) fn mpi_insert_cocomax(&mut self, slot: usize) {
        if self.machine.config.variant == MachineVariant::Coco3 {
            self.cart_error =
                Some("The CoCo Max Hi-Res Input Module requires a CoCo 1 or CoCo 2.".to_string());
            return;
        }
        if !self.mpi_flush_before_replacing_slot(slot) {
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, CoCoMaxModule::new());
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::CoCoMax;
        }
        self.power_cycle();
    }

    /// Inserts a Deluxe RS-232 Pak into MPI `slot`, starting on the inert loopback
    /// endpoint like [`CocoApp::insert_rs232`] (`launch::mount_peripherals`'s MPI arm
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
            report_rom_validation(&rom_path, &bytes);
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
