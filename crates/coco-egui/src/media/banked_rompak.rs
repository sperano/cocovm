//! Legacy `$FF40`-selected banked ROM Paks in the direct port and MPI slots.

use super::cart::CART_AUTOSTART;
use crate::*;

impl CocoApp {
    /// Loads a banked ROM Pak from `path` without installing GMC sound.
    pub(crate) fn insert_banked_rompak(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match BankedROMPak::from_bytes(&bytes, CART_AUTOSTART) {
            Ok(pak) => {
                if !self.flush_dirty_disks_or_report() {
                    return;
                }
                self.machine.insert_cartridge(pak);
                self.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.disk_rom_path = None;
                self.mpi = None;
                self.rs232 = None;
                self.rs232_eprom_path = None;
                self.rtc_direct = false;
            }
            Err(e) => self.cart_error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// Loads a banked ROM Pak into MPI `slot` without installing GMC sound.
    pub(crate) fn mpi_insert_banked_rompak(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match BankedROMPak::from_bytes(&bytes, CART_AUTOSTART) {
            Ok(pak) => {
                if !self.mpi_flush_before_replacing_slot(slot) {
                    return;
                }
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, pak);
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::BankedROMPak(path);
                }
                self.power_cycle();
            }
            Err(e) => self.cart_error = Some(format!("{}: {e}", path.display())),
        }
    }
}
