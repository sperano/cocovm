//! WD1773 floppy disk controller: the chip the FD-502 wraps. See `crate::fdc` for
//! the cartridge (`DiskCart`) that wires this to the CoCo's DSKREG latch and the
//! HALT*/NMI control lines.
//!
//! Modelled functionally rather than cycle-exact: command completion and byte
//! transfers are paced by [`WD1773::tick`] against fixed cycle counts (spec:
//! "model FUNCTIONALLY, not cycle-exact"), not the real chip's per-command timing
//! tables. Register semantics (status bit layout, side effects of register
//! access, command dispatch) are taken from the verified spec handed to this
//! implementation (MAME `wd_fdc.cpp`/`coco_fdc.cpp`).

use crate::fdc::JvcDisk;

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

/// Status register bit assignments. Bit 1 and bit 2 are reused between Type I
/// (INDEX_PULSE/TRACK0) and Type II/III (DRQ/LOST_DATA) status presentations —
/// [`WD1773::read_status`] picks the right meaning from `last_was_type1`.
pub mod status {
    pub const BUSY: u8 = 0x01;
    pub const DRQ: u8 = 0x02;
    pub const INDEX_PULSE: u8 = 0x02;
    pub const TRACK0: u8 = 0x04;
    pub const LOST_DATA: u8 = 0x04;
    pub const CRC_ERROR: u8 = 0x08;
    pub const RECORD_NOT_FOUND: u8 = 0x10;
    /// Record-type/deleted-data-mark bit (Type II read only); always 0 here —
    /// we don't model deleted-data sectors.
    pub const RECORD_TYPE: u8 = 0x20;
    pub const WRITE_PROTECT: u8 = 0x40;
    pub const NOT_READY: u8 = 0x80;
}

/// Cycles between successive DRQ byte events during a Type II/III transfer:
/// double-density byte time at ~32µs, 0.895 MHz CPU clock (spec-provided).
const DRQ_INTERVAL_CYCLES: u32 = 30;

/// Sentinel `remaining` value for a write-direction transfer waiting on the host
/// to supply the next byte via [`WD1773::write_data`] — no natural timeout fires
/// this event; only an explicit `write_data` call rearms it. Chosen so the
/// `tick` loop's `cycles.min(remaining)` never reaches zero on its own.
const AWAITING_HOST_CYCLES: u32 = u32::MAX;

/// Fixed settle delay before a Type I command (or a Type II/III not-found
/// detection) completes and raises INTRQ. Not a hardware timing figure — real
/// seeks take milliseconds and depend on the step-rate field we don't model;
/// this just keeps BUSY observably nonzero for a short, deterministic span
/// (spec: "short deterministic delay paced by tick(), functional not
/// cycle-exact").
const COMMAND_SETTLE_CYCLES: u32 = 64;

/// Delay between a read-direction transfer's LAST data-byte DRQ and command
/// completion (INTRQ): the real chip reads the sector's two CRC bytes off the
/// media first, so INTRQ trails the final DRQ by ~2 byte times. Load-bearing
/// for the FD-502 halt handshake: INTRQ clears DSKREG's halt-enable and fires
/// the NMI that ends DSKCON's transfer loop — if it rose together with the
/// final DRQ, the NMI could preempt the `LDA $FF4B` that collects the last
/// byte of every sector.
const CRC_TRAILER_CYCLES: u32 = 2 * DRQ_INTERVAL_CYCLES;

/// Bytes delivered by a Type III Read Address command: track, side, sector,
/// size code, CRC1, CRC2.
const READ_ADDRESS_LEN: usize = 6;

/// Bytes a Type III Write Track (format) command consumes before completing —
/// a 300 RPM double-density (250 kbit/s MFM) track's approximate byte count
/// (spec-provided). Parsing the format stream is not implemented (milestone 1
/// scope): the bytes are discarded and no sector layout is written.
/// TODO: parse the format stream (gap/sync/ID/data fields) once formatting
/// from within the emulator is needed; today only pre-built JVC images are
/// supported.
const WRITE_TRACK_BYTE_COUNT: usize = 6250;

/// Which family of Type I step commands last ran, so a bare "Step" (no
/// direction of its own) repeats the last Step-In/Step-Out direction — the
/// WD1773 datasheet's documented behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferKind {
    ReadSector,
    WriteSector,
    ReadAddress,
    WriteTrack,
}

/// An in-progress byte-paced data transfer (Type II Read/Write Sector, Type III
/// Read Address/Write Track).
#[derive(Debug, Clone)]
struct Transfer {
    kind: TransferKind,
    /// Cycles remaining until the next DRQ event ([`AWAITING_HOST_CYCLES`] while
    /// a write-direction transfer waits on [`WD1773::write_data`]).
    remaining: u32,
    /// Next byte index within the current sector/ID-field/format run.
    index: usize,
    /// Total bytes in the current sector/ID-field/format run.
    total: usize,
    /// Type II 'm' bit: after this sector, continue to the next one.
    multiple: bool,
    /// Byte offset of the current sector in the disk image (Read/Write Sector
    /// only; unused for Read Address/Write Track).
    offset: usize,
    /// Read-direction bytes staged for DRQ delivery (Read Sector/Read Address);
    /// empty for writes and Write Track.
    buf: Vec<u8>,
    /// True only for the very first byte of a freshly-dispatched command (not
    /// set on a multiple-sector continuation's first byte). `drq`'s reset
    /// default is `true` (for the HALT-line reason documented on
    /// [`WD1773::drq`]), which is stale leftover state, not a real unread
    /// byte from a previous transfer — this flag keeps that default from
    /// spuriously flagging LOST DATA on a command's first delivered byte.
    first_byte: bool,
}

/// What the controller is doing between command dispatch and completion.
#[derive(Debug, Clone)]
enum Op {
    Idle,
    /// A Type I command settling before INTRQ ([`COMMAND_SETTLE_CYCLES`]).
    SettlingTypeOne { remaining: u32 },
    /// A Type II/III command whose target track/sector/side isn't on the
    /// mounted image: RNF fires after the same short settle.
    SettlingNotFound { remaining: u32 },
    Transfer(Transfer),
}

/// The WD1773 chip: registers, command state machine, DRQ/INTRQ lines.
///
/// Owns no disk state itself — [`WD1773::write_command`], [`WD1773::tick`], and
/// [`WD1773::write_data`] take the currently-selected drive's [`JvcDisk`] (or
/// `None`) and the DSKREG-derived side select as parameters, so the caller
/// (`crate::fdc::DiskCart`) owns drive selection and the four drive slots.
#[derive(Debug, Clone)]
pub struct WD1773 {
    /// Track register: the controller's belief of the current track (what
    /// Restore/Seek/verified-Step commands leave it at).
    pub track: u8,
    pub sector: u8,
    pub data: u8,
    pub busy: bool,
    /// Data request line. Reset state is `true` (spec) — see `crate::fdc`'s
    /// HALT-line doc comment for why: DSKREG's halt-enable bit is asserted
    /// asynchronously by boot code before any command has run, and a
    /// default-`false` DRQ would spuriously assert HALT.
    pub drq: bool,
    pub intrq: bool,
    /// Actual head position, distinct from `track` when a Step/Step-In/Step-Out
    /// command runs with its T (update-track-register) bit clear: the head
    /// still moves, but the visible track register doesn't follow. Read/Write
    /// Sector and Read Address operate on this, not on `track`.
    physical_track: u8,
    /// Direction a bare "Step" (no direction of its own) repeats.
    last_step_direction: StepDirection,
    /// True while the last-dispatched (or currently-settling) command was Type
    /// I, or immediately after a Force Interrupt — selects which meaning bits 1
    /// and 2 of the status byte have.
    last_was_type1: bool,
    status_lost_data: bool,
    status_crc_error: bool,
    status_record_not_found: bool,
    status_write_protect: bool,
    op: Op,
}

impl Default for WD1773 {
    fn default() -> Self {
        Self {
            track: 0,
            sector: 0,
            data: 0,
            busy: false,
            drq: true,
            intrq: false,
            physical_track: 0,
            last_step_direction: StepDirection::In,
            last_was_type1: true,
            status_lost_data: false,
            status_crc_error: false,
            status_record_not_found: false,
            status_write_protect: false,
            op: Op::Idle,
        }
    }
}

impl WD1773 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the status register ($FF48 on the CoCo). Side effect: clears INTRQ
    /// (MAME `wd_fdc.cpp:1299`).
    pub fn read_status(&mut self, disk_present: bool, motor_on: bool) -> u8 {
        let s = self.status_byte(disk_present, motor_on);
        self.intrq = false;
        s
    }

    fn status_byte(&self, disk_present: bool, motor_on: bool) -> u8 {
        let mut s = 0u8;
        if self.busy {
            s |= status::BUSY;
        }
        if self.last_was_type1 {
            if self.physical_track == 0 {
                s |= status::TRACK0;
            }
            // INDEX_PULSE (bit1) intentionally left 0: Disk BASIC doesn't need
            // it (spec).
        } else {
            if self.drq {
                s |= status::DRQ;
            }
            if self.status_lost_data {
                s |= status::LOST_DATA;
            }
        }
        if self.status_crc_error {
            s |= status::CRC_ERROR;
        }
        if self.status_record_not_found {
            s |= status::RECORD_NOT_FOUND;
        }
        if self.status_write_protect {
            s |= status::WRITE_PROTECT;
        }
        if !disk_present || !motor_on {
            s |= status::NOT_READY;
        }
        s
    }

    /// Read the data register ($FF4B). Side effect: clears DRQ.
    pub fn read_data(&mut self) -> u8 {
        let val = self.data;
        self.drq = false;
        val
    }

    /// Write the data register ($FF4B). Side effect: clears DRQ, and — mid a
    /// Write Sector/Write Track transfer — supplies the next byte, advancing
    /// (and possibly completing) the transfer.
    pub fn write_data(&mut self, val: u8, mut disk: Option<&mut JvcDisk>, side: u8) {
        self.data = val;
        self.drq = false;
        let Op::Transfer(mut t) = std::mem::replace(&mut self.op, Op::Idle) else {
            return;
        };
        if !matches!(t.kind, TransferKind::WriteSector | TransferKind::WriteTrack) {
            self.op = Op::Transfer(t);
            return;
        }
        if t.index < t.total {
            if t.kind == TransferKind::WriteSector
                && let Some(d) = disk.as_deref_mut()
            {
                d.write_byte(t.offset + t.index, val);
            }
            t.index += 1;
        }
        if t.index >= t.total {
            self.finish_transfer(t, disk, side);
        } else {
            t.remaining = DRQ_INTERVAL_CYCLES;
            self.op = Op::Transfer(t);
        }
    }

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
                self.start_write_track();
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
        let multiple = cmd & type1::UPDATE_TRACK_REG != 0; // bit4, same physical bit as T
        match disk {
            Some(d) => match d.sector_offset(self.physical_track, side, self.sector) {
                Some(offset) => {
                    let total = d.sector_size();
                    let buf = d.read_bytes(offset, total).to_vec();
                    self.op = Op::Transfer(Transfer {
                        kind: TransferKind::ReadSector,
                        remaining: DRQ_INTERVAL_CYCLES,
                        index: 0,
                        total,
                        multiple,
                        offset,
                        buf,
                        first_byte: true,
                    });
                }
                None => self.start_not_found(),
            },
            None => self.start_not_found(),
        }
    }

    /// Write Sector (Type II, `0xA`/`0xB`).
    fn start_write_sector(&mut self, cmd: u8, disk: Option<&mut JvcDisk>, side: u8) {
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
                        remaining: DRQ_INTERVAL_CYCLES,
                        index: 0,
                        total,
                        multiple,
                        offset,
                        buf: Vec::new(),
                        first_byte: true,
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
                    remaining: DRQ_INTERVAL_CYCLES,
                    index: 0,
                    total: READ_ADDRESS_LEN,
                    multiple: false,
                    offset: 0,
                    buf,
                    first_byte: true,
                });
            }
            _ => self.start_not_found(),
        }
    }

    /// Write Track (Type III, `0xF`, format): consumes [`WRITE_TRACK_BYTE_COUNT`]
    /// DRQ-paced bytes and discards them — see that constant's doc comment.
    fn start_write_track(&mut self) {
        self.op = Op::Transfer(Transfer {
            kind: TransferKind::WriteTrack,
            remaining: DRQ_INTERVAL_CYCLES,
            index: 0,
            total: WRITE_TRACK_BYTE_COUNT,
            multiple: false,
            offset: 0,
            buf: Vec::new(),
            first_byte: true,
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

    /// Advance the command state machine by `cycles` CPU cycles. `disk`/`side`
    /// are the currently-selected drive (per DSKREG) and its side select —
    /// needed for the not-found detection and for locating the next sector of
    /// a multiple-sector transfer.
    pub fn tick(&mut self, mut cycles: u32, mut disk: Option<&mut JvcDisk>, side: u8) {
        while cycles > 0 {
            let consumed = match &mut self.op {
                Op::Idle => break,
                Op::SettlingTypeOne { remaining } | Op::SettlingNotFound { remaining } => {
                    let step = cycles.min(*remaining);
                    *remaining -= step;
                    if *remaining == 0 {
                        self.busy = false;
                        self.intrq = true;
                        self.op = Op::Idle;
                    }
                    step
                }
                Op::Transfer(t) => {
                    let step = cycles.min(t.remaining);
                    t.remaining -= step;
                    if t.remaining == 0 {
                        self.advance_transfer(disk.as_deref_mut(), side);
                    }
                    step
                }
            };
            if consumed == 0 {
                break;
            }
            cycles -= consumed;
        }
    }

    /// A DRQ interval elapsed mid-transfer: deliver the next read byte, or (for
    /// writes) request one and wait for [`WD1773::write_data`].
    fn advance_transfer(&mut self, disk: Option<&mut JvcDisk>, side: u8) {
        let Op::Transfer(mut t) = std::mem::replace(&mut self.op, Op::Idle) else {
            unreachable!("advance_transfer only called from the Op::Transfer arm");
        };
        match t.kind {
            TransferKind::ReadSector | TransferKind::ReadAddress => {
                if t.index >= t.total {
                    // The CRC trailer elapsed after the final data byte; a
                    // still-unread final byte is a genuine overrun.
                    if self.drq {
                        self.status_lost_data = true;
                    }
                    self.finish_transfer(t, disk, side);
                    return;
                }
                // Spec: "if the previous byte was never taken, set LOST
                // DATA but keep going (do not stall)." Exempt only the
                // very first byte of a fresh command — see `first_byte`.
                if self.drq && !t.first_byte {
                    self.status_lost_data = true;
                }
                self.data = t.buf[t.index];
                self.drq = true;
                t.index += 1;
                t.first_byte = false;
                // After the final data byte, INTRQ waits out the CRC trailer
                // (see [`CRC_TRAILER_CYCLES`]) so the host can collect the
                // byte before completion clears halt-enable and fires NMI.
                t.remaining = if t.index >= t.total {
                    CRC_TRAILER_CYCLES
                } else {
                    DRQ_INTERVAL_CYCLES
                };
                self.op = Op::Transfer(t);
            }
            TransferKind::WriteSector | TransferKind::WriteTrack => {
                // Request the next byte; write_data() drives the transfer
                // forward from here, so there's no natural timeout.
                self.drq = true;
                t.remaining = AWAITING_HOST_CYCLES;
                self.op = Op::Transfer(t);
            }
        }
    }

    /// A sector/ID-field/format run finished. For a multiple-sector Type II
    /// command, roll onto the next sector (RNF once one runs past the end of
    /// the track); otherwise complete with INTRQ.
    fn finish_transfer(&mut self, t: Transfer, disk: Option<&mut JvcDisk>, side: u8) {
        if t.multiple {
            let next_sector = self.sector.wrapping_add(1);
            if let Some(d) = disk
                && let Some(offset) = d.sector_offset(self.physical_track, side, next_sector)
            {
                self.sector = next_sector;
                let total = d.sector_size();
                let buf = match t.kind {
                    TransferKind::ReadSector => d.read_bytes(offset, total).to_vec(),
                    _ => Vec::new(),
                };
                self.op = Op::Transfer(Transfer {
                    kind: t.kind,
                    remaining: DRQ_INTERVAL_CYCLES,
                    index: 0,
                    total,
                    multiple: true,
                    offset,
                    buf,
                    // Not exempted: this continues the same multiple-sector
                    // transfer, so a still-unread last byte of the previous
                    // sector is a genuine overrun (spec's LOST DATA case).
                    first_byte: false,
                });
                return;
            }
            self.status_record_not_found = true;
        }
        self.busy = false;
        self.intrq = true;
        self.op = Op::Idle;
    }
}
