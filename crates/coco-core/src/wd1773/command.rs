//! Command register ($FF48 write) dispatch: decodes the WD1773's four
//! command types and starts the corresponding [`Op`](super::Op)/
//! [`Transfer`](super::Transfer). See [`WD1773::write_command`].

use crate::fdc::JvcDisk;

use super::{
    COMMAND_SETTLE_CYCLES, DRQ_INTERVAL_CYCLES, FIRST_BYTE_LATENCY_CYCLES, FormatState, Op,
    READ_ADDRESS_LEN, StepDirection, Transfer, TransferKind, WD1773, WRITE_TRACK_BYTE_COUNT,
};

/// Command byte top-nibble values (`cmd >> 4`). The paired members of a family
/// (Step/Step-In/Step-Out's T flag, Read/Write Sector's m flag) differ only in
/// the nibble's low bit — see the doc comments on the affected `start_*` methods.
mod cmd_type {
    pub const RESTORE: u8 = 0x0;
    pub const SEEK: u8 = 0x1;
    pub const STEP: u8 = 0x2;
    pub const STEP_T: u8 = 0x3;
    pub const STEP_IN: u8 = 0x4;
    pub const STEP_IN_T: u8 = 0x5;
    pub const STEP_OUT: u8 = 0x6;
    pub const STEP_OUT_T: u8 = 0x7;
    pub const READ_SECTOR: u8 = 0x8;
    pub const READ_SECTOR_M: u8 = 0x9;
    pub const WRITE_SECTOR: u8 = 0xA;
    pub const WRITE_SECTOR_M: u8 = 0xB;
    pub const READ_ADDRESS: u8 = 0xC;
    pub const FORCE_INTERRUPT: u8 = 0xD;
    pub const READ_TRACK: u8 = 0xE;
    pub const WRITE_TRACK: u8 = 0xF;
}

/// Type I (Restore/Seek/Step*) command low-nibble bits.
mod type1 {
    /// Verify: after the seek/step, confirm the target track has a readable ID
    /// (i.e. is within the mounted image's track count); else set RNF.
    pub const VERIFY: u8 = 0x04;
    /// Step/Step-In/Step-Out only: update the track register to the new
    /// position. Restore and Seek always update it regardless of this bit
    /// (there is no non-updating variant of either).
    pub const UPDATE_TRACK_REG: u8 = 0x10;
}

/// Type IV (Force Interrupt) low-nibble interrupt-condition bits.
mod type4 {
    /// I3: force an immediate INTRQ.
    pub const IMMEDIATE_INTRQ: u8 = 0x08;
}

impl WD1773 {
    /// Write the command register ($FF48). Force Interrupt (Type IV) runs even
    /// while busy (it's how you cancel a stuck command); every other command
    /// written while busy is ignored (spec).
    pub fn write_command(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
        let type_nibble = cmd >> 4;
        if type_nibble == cmd_type::FORCE_INTERRUPT {
            self.force_interrupt(cmd);
            return;
        }
        if self.busy {
            return;
        }
        self.busy = true;
        self.status_lost_data = false;
        self.status_crc_error = false;
        self.status_record_not_found = false;
        self.status_write_protect = false;
        match type_nibble {
            cmd_type::RESTORE => self.start_restore(cmd, disk),
            cmd_type::SEEK => self.start_seek(cmd, disk),
            cmd_type::STEP | cmd_type::STEP_T => self.start_step(cmd, None, disk),
            cmd_type::STEP_IN | cmd_type::STEP_IN_T => {
                self.start_step(cmd, Some(StepDirection::In), disk)
            }
            cmd_type::STEP_OUT | cmd_type::STEP_OUT_T => {
                self.start_step(cmd, Some(StepDirection::Out), disk)
            }
            cmd_type::READ_SECTOR | cmd_type::READ_SECTOR_M => {
                self.last_was_type1 = false;
                self.start_read_sector(cmd, disk, side)
            }
            cmd_type::WRITE_SECTOR | cmd_type::WRITE_SECTOR_M => {
                self.last_was_type1 = false;
                self.start_write_sector(cmd, disk, side)
            }
            cmd_type::READ_ADDRESS => {
                self.last_was_type1 = false;
                self.start_read_address(disk, side)
            }
            cmd_type::READ_TRACK => {
                // "Optional, RNF is acceptable" (spec) — we don't model track
                // reads at all, so every Read Track is a not-found.
                self.last_was_type1 = false;
                self.start_not_found();
            }
            cmd_type::WRITE_TRACK => {
                self.last_was_type1 = false;
                self.start_write_track(disk);
            }
            _ => unreachable!("4-bit nibble: all 16 values are matched above"),
        }
    }

    fn track_readable(disk: Option<&JvcDisk>, track: u8) -> bool {
        disk.is_some_and(|d| (track as usize) < d.track_count())
    }

    /// Restore (Type I, `0x0`): seek to physical track 0.
    fn start_restore(&mut self, cmd: u8, disk: Option<&mut JvcDisk>) {
        self.last_was_type1 = true;
        self.physical_track = 0;
        self.track = 0;
        let verify = cmd & type1::VERIFY != 0;
        self.status_record_not_found = verify && !Self::track_readable(disk.as_deref(), 0);
        self.op = Op::SettlingTypeOne { remaining: COMMAND_SETTLE_CYCLES };
    }

    /// Seek (Type I, `0x1`): move to the track named by the data register.
    fn start_seek(&mut self, cmd: u8, disk: Option<&mut JvcDisk>) {
        self.last_was_type1 = true;
        let target = self.data;
        self.physical_track = target;
        self.track = target;
        let verify = cmd & type1::VERIFY != 0;
        self.status_record_not_found = verify && !Self::track_readable(disk.as_deref(), target);
        self.op = Op::SettlingTypeOne { remaining: COMMAND_SETTLE_CYCLES };
    }

    /// Step/Step-In/Step-Out (Type I, `0x2`-`0x7`): move one track in
    /// `forced_direction` (Step-In/Step-Out) or the last remembered direction
    /// (bare Step), updating the track register only if the command's T bit
    /// ([`type1::UPDATE_TRACK_REG`]) is set.
    fn start_step(
        &mut self,
        cmd: u8,
        forced_direction: Option<StepDirection>,
        disk: Option<&mut JvcDisk>,
    ) {
        self.last_was_type1 = true;
        if let Some(dir) = forced_direction {
            self.last_step_direction = dir;
        }
        let new_track = match self.last_step_direction {
            StepDirection::In => self.physical_track.saturating_add(1),
            StepDirection::Out => self.physical_track.saturating_sub(1),
        };
        self.physical_track = new_track;
        if cmd & type1::UPDATE_TRACK_REG != 0 {
            self.track = new_track;
        }
        let verify = cmd & type1::VERIFY != 0;
        self.status_record_not_found =
            verify && !Self::track_readable(disk.as_deref(), new_track);
        self.op = Op::SettlingTypeOne { remaining: COMMAND_SETTLE_CYCLES };
    }

    /// Read Sector (Type II, `0x8`/`0x9`).
    fn start_read_sector(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
        // No data yet: DRQ low so a HALT-enabled driver stalls at its LDA
        // DATAREG loop until the first byte lands (see FIRST_BYTE_LATENCY_CYCLES).
        self.drq = false;
        let multiple = cmd & type1::UPDATE_TRACK_REG != 0; // bit4, same physical bit as T
        match disk {
            Some(d) => match d.sector_offset(self.physical_track, side, self.sector) {
                Some(offset) => {
                    let total = d.sector_size();
                    let buf = d.read_bytes(offset, total).to_vec();
                    self.op = Op::Transfer(Transfer {
                        kind: TransferKind::ReadSector,
                        remaining: FIRST_BYTE_LATENCY_CYCLES,
                        index: 0,
                        total,
                        multiple,
                        offset,
                        buf,
                        first_byte: true,
                        format_state: FormatState::Gap,
                        last_id_field: None,
                        format_enabled: true,
                    });
                }
                None => self.start_not_found(),
            },
            None => self.start_not_found(),
        }
    }

    /// Write Sector (Type II, `0xA`/`0xB`).
    fn start_write_sector(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
        // No sector located yet: DRQ low until the ID field is found and the
        // controller requests the first byte (see FIRST_BYTE_LATENCY_CYCLES).
        self.drq = false;
        let multiple = cmd & type1::UPDATE_TRACK_REG != 0;
        match disk {
            Some(d) if d.write_protected() => {
                self.status_write_protect = true;
                self.busy = false;
                self.intrq = true;
                self.op = Op::Idle;
            }
            Some(d) => match d.sector_offset(self.physical_track, side, self.sector) {
                Some(offset) => {
                    let total = d.sector_size();
                    self.op = Op::Transfer(Transfer {
                        kind: TransferKind::WriteSector,
                        remaining: FIRST_BYTE_LATENCY_CYCLES,
                        index: 0,
                        total,
                        multiple,
                        offset,
                        buf: Vec::new(),
                        first_byte: true,
                        format_state: FormatState::Gap,
                        last_id_field: None,
                        format_enabled: true,
                    });
                }
                None => self.start_not_found(),
            },
            None => self.start_not_found(),
        }
    }

    /// Read Address (Type III, `0xC`): deliver the 6 ID bytes (track, side,
    /// first sector ID on the track, size code, CRC1, CRC2 — CRC bytes are 0,
    /// unmodelled). The sector register is deliberately left alone (spec: "not
    /// needed").
    fn start_read_address(&mut self, disk: Option<&mut JvcDisk>, side: u8) {
        // No ID field under the head yet: DRQ low until the next address mark
        // spins around (see FIRST_BYTE_LATENCY_CYCLES).
        self.drq = false;
        match disk {
            Some(d) if (self.physical_track as usize) < d.track_count() => {
                let buf = vec![
                    self.physical_track,
                    side,
                    d.first_sector_id(),
                    d.size_code(),
                    0,
                    0,
                ];
                self.op = Op::Transfer(Transfer {
                    kind: TransferKind::ReadAddress,
                    remaining: FIRST_BYTE_LATENCY_CYCLES,
                    index: 0,
                    total: READ_ADDRESS_LEN,
                    multiple: false,
                    offset: 0,
                    buf,
                    first_byte: true,
                    format_state: FormatState::Gap,
                    last_id_field: None,
                    format_enabled: true,
                });
            }
            _ => self.start_not_found(),
        }
    }

    /// Write Track (Type III, `0xF`, format): consumes
    /// [`WRITE_TRACK_BYTE_COUNT`] DRQ-paced bytes, parsing them into sectors
    /// laid onto the mounted image via
    /// [`feed_write_track_byte`](super::transfer::feed_write_track_byte) when
    /// the controller is in double density (FM streams are still just
    /// discarded — see that constant's doc comment). Write-protect is checked
    /// up front, mirroring [`WD1773::start_write_sector`]'s WP arm; there's no
    /// "not found" case since a format command has no target sector to fail
    /// to find (spec).
    fn start_write_track(&mut self, disk: Option<&mut JvcDisk>) {
        if let Some(d) = disk
            && d.write_protected()
        {
            self.status_write_protect = true;
            self.busy = false;
            self.intrq = true;
            self.op = Op::Idle;
            return;
        }
        self.op = Op::Transfer(Transfer {
            kind: TransferKind::WriteTrack,
            remaining: DRQ_INTERVAL_CYCLES,
            index: 0,
            total: WRITE_TRACK_BYTE_COUNT,
            multiple: false,
            offset: 0,
            buf: Vec::new(),
            first_byte: true,
            format_state: FormatState::Gap,
            last_id_field: None,
            format_enabled: self.density_double,
        });
    }

    fn start_not_found(&mut self) {
        self.status_record_not_found = true;
        self.op = Op::SettlingNotFound { remaining: COMMAND_SETTLE_CYCLES };
    }

    /// Force Interrupt (Type IV, `0xD`): cancel any command in progress. Low
    /// nibble bit3 (I3) forces an immediate INTRQ; low nibble 0 just cancels.
    /// Runs even while busy, and the status presented afterward is Type-I
    /// style (spec).
    fn force_interrupt(&mut self, cmd: u8) {
        self.busy = false;
        self.op = Op::Idle;
        self.last_was_type1 = true;
        if cmd & type4::IMMEDIATE_INTRQ != 0 {
            self.intrq = true;
        }
    }
}
