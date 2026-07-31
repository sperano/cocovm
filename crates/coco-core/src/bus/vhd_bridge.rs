//! `$FF80-$FF86` virtual-hard-disk command execution (NitrOS-9 `emudsk`):
//! the `$FF83` command dispatch and the READ/WRITE/FLUSH bodies, which move
//! sector data through the CPU's own logical address space.

use mc6809::Bus;

use crate::vhd;

use super::SystemBus;

impl SystemBus {
    /// `$FF83` write: execute a VHD command on the selected drive
    /// (`vhd::command`), synchronously, then latch the resulting status for
    /// the next `$FF83` read.
    ///
    /// Order, per spec: no drive selected -> the write is a no-op entirely
    /// (no status changes anywhere). Reentrant call (from within our own
    /// transfer loop below, via a buffer address that lands back on this same
    /// register) -> also a no-op, so the outer call's result isn't clobbered.
    /// Otherwise: an unmounted drive always reports `NO_VHD`, regardless of
    /// which command was written; only a mounted drive dispatches on the
    /// command byte.
    pub(super) fn vhd_execute_command(&mut self, cmd: u8) {
        let Some(drive) = self.vhd.selected_drive() else {
            return;
        };
        if self.vhd.busy {
            return;
        }
        self.vhd.busy = true;

        if self.vhd.drives[drive].image.is_none() {
            self.vhd.drives[drive].status = vhd::status::NO_VHD;
        } else {
            // Every real dispatch (as opposed to an unknown command byte)
            // counts as one access, for the status bar's activity light.
            if matches!(
                cmd,
                vhd::command::READ | vhd::command::WRITE | vhd::command::FLUSH
            ) {
                self.vhd.access_counts[drive] += 1;
            }
            match cmd {
                vhd::command::READ => self.vhd_read_sector(drive),
                vhd::command::WRITE => self.vhd_write_sector(drive),
                vhd::command::FLUSH => self.vhd_flush(drive),
                _ => self.vhd.drives[drive].status = vhd::status::UNKNOWN_COMMAND,
            }
        }

        self.vhd.busy = false;
    }

    /// READ (`vhd::command::READ`): fetch the sector at `drive`'s LRN from
    /// its image (zero-padding any short/EOF tail) and transfer all
    /// [`vhd::SECTOR_SIZE`] bytes to `drive`'s buffer address through the
    /// CPU's logical address space (MMU-translated, one byte at a time,
    /// wrapping at 64K) — exactly the path real CPU-driven code would take.
    fn vhd_read_sector(&mut self, drive: usize) {
        let offset = vhd_sector_offset(self.vhd.drives[drive].lrn);
        let mut buf = [0u8; vhd::SECTOR_SIZE];
        let read_result = self.vhd_image_mut(drive).read_at(offset, &mut buf);
        match read_result {
            Ok(_) => {
                let buffer_addr = self.vhd.drives[drive].buffer_addr;
                for (i, byte) in buf.iter().enumerate() {
                    self.write(buffer_addr.wrapping_add(i as u16), *byte);
                }
                self.vhd.drives[drive].status = vhd::status::OK;
            }
            Err(_) => self.vhd.drives[drive].status = vhd::status::IO_ERROR,
        }
    }

    /// WRITE (`vhd::command::WRITE`): zero-extend the image up to `drive`'s
    /// LRN offset first, THEN fetch [`vhd::SECTOR_SIZE`] bytes from `drive`'s
    /// buffer address through the CPU's logical address space, THEN write
    /// them into the image. This exact order matters: MAME performs the
    /// zero-extend before touching the CPU bus, which is observable if the
    /// buffer address happens to overlap the VHD's own I/O registers.
    fn vhd_write_sector(&mut self, drive: usize) {
        let offset = vhd_sector_offset(self.vhd.drives[drive].lrn);
        if self.vhd_image_mut(drive).extend_to(offset).is_err() {
            self.vhd.drives[drive].status = vhd::status::IO_ERROR;
            return;
        }

        let buffer_addr = self.vhd.drives[drive].buffer_addr;
        let mut buf = [0u8; vhd::SECTOR_SIZE];
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = self.read(buffer_addr.wrapping_add(i as u16));
        }

        let write_result = self.vhd_image_mut(drive).write_at(offset, &buf);
        self.vhd.drives[drive].status = if write_result.is_ok() {
            vhd::status::OK
        } else {
            vhd::status::IO_ERROR
        };
    }

    /// FLUSH (`vhd::command::FLUSH`): flush the backing file to disk. Mapping
    /// a flush I/O error to `IO_ERROR` (like read/write) is this
    /// implementation's own extension, not a separately verified MAME fact.
    fn vhd_flush(&mut self, drive: usize) {
        let flush_result = self.vhd_image_mut(drive).flush();
        self.vhd.drives[drive].status = if flush_result.is_ok() {
            vhd::status::OK
        } else {
            vhd::status::IO_ERROR
        };
    }

    /// The image mounted in `drive`, for the command bodies above. Panics if
    /// called on an unmounted drive — every call site is guarded by
    /// `vhd_execute_command`'s own mounted check first.
    fn vhd_image_mut(&mut self, drive: usize) -> &mut vhd::VHDImage {
        self.vhd.drives[drive]
            .image
            .as_mut()
            .expect("checked mounted")
    }
}

/// Byte offset of logical record `lrn` within a VHD image ([`vhd::SECTOR_SIZE`]
/// bytes/sector). `u64` arithmetic avoids overflow even though `lrn` is only
/// ever up to 24 bits wide.
fn vhd_sector_offset(lrn: u32) -> u64 {
    vhd::SECTOR_SIZE as u64 * u64::from(lrn)
}
