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
//!
//! Command dispatch ($FF48 write) lives in the [`command`] submodule; byte-paced
//! transfer handling ($FF4B read/write, the DRQ advance loop, and the Write
//! Track MFM parser) lives in [`transfer`].

use serde::{Deserialize, Serialize};

use crate::fdc::JVCDisk;

mod command;
mod transfer;

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

/// MFM byte times a Read/Write Sector command spends searching from the current
/// head position to the target sector's DATA field before the first byte is
/// available: the ID address mark, its 4-byte ID field and 2 CRC bytes, the
/// ~22-byte Gap 2, and the data address mark. This is the *minimum* — a real
/// rotation adds up to a full revolution on top — but it is already far longer
/// than the microseconds-scale setup a driver runs between writing the command
/// and enabling its byte-transfer handshake.
///
/// Load-bearing for polled/HALT drivers that issue the command, run a short
/// fixed delay, THEN arm the transfer (NitrOS-9 `boot_1773`'s ~54-cycle
/// `Delay2` before it sets HALT-enable and enters its `LDA DATAREG` loop). If
/// the first DRQ fires during that delay window the driver never collects those
/// bytes and the transfer trips LOST DATA — which `boot_1773`'s NMI handler
/// reads as `E$Read` and the boot fails. Pacing the *first* byte by one
/// [`DRQ_INTERVAL_CYCLES`] (as every earlier command did) put it inside the
/// window; DSKCON only escaped because it arms HALT before issuing the command.
const FIRST_SECTOR_SEARCH_BYTES: u32 = 30;
const FIRST_BYTE_LATENCY_CYCLES: u32 = FIRST_SECTOR_SEARCH_BYTES * DRQ_INTERVAL_CYCLES;

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
/// headroom above DSKINI 1.1's own 6280-byte double-density track template
/// (32×$4E gap1 + 18 sectors' ID/gap2/sync/data/gap3 fields + 200×$4E gap4;
/// spec-provided). The stream IS parsed in double density (see `wd1773::transfer`'s
/// `mod mfm`, [`FormatState`], `feed_write_track_byte`) to lay sectors into the
/// mounted image; FM format streams are still only consumed and discarded
/// (FM parsing is unimplemented — density comes from `WD1773`'s
/// `density_double` field, set via [`WD1773::set_double_density`]).
const WRITE_TRACK_BYTE_COUNT: usize = 6400;

/// Which family of Type I step commands last ran, so a bare "Step" (no
/// direction of its own) repeats the last Step-In/Step-Out direction — the
/// WD1773 datasheet's documented behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum StepDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TransferKind {
    ReadSector,
    WriteSector,
    ReadAddress,
    WriteTrack,
}

/// Write Track (format) mark-triggered parser state: scans the incoming
/// byte stream for MFM address marks framed by `$F5` sync runs and
/// terminated by `$F7` (see `wd1773::transfer`'s `mod mfm` and
/// `feed_write_track_byte`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum FormatState {
    /// Skipping gap/filler bytes, waiting for a `$F5` sync run.
    Gap,
    /// At least one `$F5` seen; the next non-`$F5` byte is the address mark.
    Sync,
    /// `$FE` seen: still gathering the 4 literal ID bytes (track, side, sector, size).
    IdField(Vec<u8>),
    /// The 4 ID bytes are gathered; consuming (ignored) bytes until `$F7`.
    IdFieldTerm { track: u8, sector: u8, size_code: u8 },
    /// `$FB`/`$F8` seen: still gathering the sector's data payload (target length also carried).
    DataField(Vec<u8>, usize),
    /// The payload is fully gathered; consuming (ignored) bytes until `$F7`.
    DataFieldTerm(Vec<u8>),
}

/// An in-progress byte-paced data transfer (Type II Read/Write Sector, Type III
/// Read Address/Write Track).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Write Track's mark-triggered parser state; unused (stays [`FormatState::Gap`])
    /// for every other `kind`.
    format_state: FormatState,
    /// (track, sector, size_code) of the most recently completed ID field
    /// during a Write Track transfer; `None` until the first one completes,
    /// and for every other `kind`.
    last_id_field: Option<(u8, u8, u8)>,
    /// True if a Write Track transfer should run the MFM format-stream parser
    /// (double density at dispatch time) rather than discard bytes (FM).
    /// Never consulted outside the `WriteTrack` arms.
    format_enabled: bool,
}

/// What the controller is doing between command dispatch and completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// [`WD1773::write_data`] take the currently-selected drive's [`JVCDisk`] (or
/// `None`) and the DSKREG-derived side select as parameters, so the caller
/// (`crate::fdc::DiskCart`) owns drive selection and the four drive slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Density the controller currently operates at, from DSKREG bit5
    /// (`dskreg::DENSITY_AND_NMI_ENABLE`, set out-of-band via
    /// [`WD1773::set_double_density`] — see `crate::fdc`). Gates whether
    /// Write Track parses the MFM format stream (`true`) or discards it (FM,
    /// `false`, unimplemented). Defaults `true` so direct-construction unit
    /// tests that never call the setter still exercise the MFM/parsing path.
    density_double: bool,
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
            density_double: true,
        }
    }
}

impl WD1773 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the density the controller currently operates at (DSKREG bit5,
    /// `dskreg::DENSITY_AND_NMI_ENABLE` — see `crate::fdc::DiskCart::write`,
    /// which calls this before every `write_command`). Determines whether a
    /// subsequently-dispatched Write Track runs the MFM format-stream parser
    /// (`true`) or falls back to discard-only behavior (`false`, FM — parsing
    /// FM format streams is unimplemented).
    pub fn set_double_density(&mut self, double_density: bool) {
        self.density_double = double_density;
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

    /// Restore-only structural check, independent of any mounted disk: an
    /// in-flight Read Sector/Read Address transfer's `index` must not exceed
    /// its own `buf` — [`WD1773::advance_transfer`] indexes `t.buf[t.index]`
    /// once `index < total` (already guaranteed by construction, but `index`/
    /// `total`/`buf` are all ordinary deserialized fields a hand-crafted
    /// payload can desync from each other). Write Sector/Write Track never
    /// read from `buf` at all — they write straight through to the disk
    /// image (`WD1773::write_data`) or discard, leaving `buf` empty
    /// (`Vec::new()`) for the whole transfer — so `index` legitimately
    /// exceeds `buf.len()` (0) for those two kinds mid transfer; not checked
    /// here. See [`crate::fdc::DiskCart::validate_restored_transfer`] for the
    /// disk-bound half of this check, which needs a reattached
    /// [`JVCDisk`] and so runs later in the restore flow.
    pub(crate) fn validate_restored(&self) -> Result<(), String> {
        let Op::Transfer(t) = &self.op else { return Ok(()) };
        if matches!(t.kind, TransferKind::ReadSector | TransferKind::ReadAddress) && t.index > t.buf.len()
        {
            return Err(format!(
                "Transfer.index ({}) exceeds Transfer.buf length ({}) for a {:?} transfer",
                t.index,
                t.buf.len(),
                t.kind
            ));
        }
        Ok(())
    }

    /// Restore-only, called AFTER floppy reattachment
    /// ([`crate::fdc::DiskCart::validate_restored_transfer`]): bound-check an
    /// in-flight Read/Write Sector transfer's `offset`/`total` against
    /// `disk`'s actual reattached byte length. `offset`/`total` are ordinary
    /// deserialized fields a hand-crafted payload can set to anything;
    /// [`JVCDisk::write_byte`]/[`JVCDisk::read_bytes`] index straight into
    /// `data` with no bounds check of their own, so an out-of-range pair
    /// would panic the instant the transfer resumes
    /// (`docs/plan-save-states.md`). Read Address/Write Track transfers
    /// never index `data` by `offset` at all (Read Address's `buf` is a
    /// fixed 6-byte reply built at dispatch time; Write Track lays sectors
    /// via [`JVCDisk::format_sector`], which computes its own bounded
    /// offset), so only the two sector-transfer kinds are checked.
    pub(crate) fn validate_transfer_bounds(&self, disk: Option<&JVCDisk>) -> Result<(), String> {
        let Op::Transfer(t) = &self.op else { return Ok(()) };
        if !matches!(t.kind, TransferKind::ReadSector | TransferKind::WriteSector) {
            return Ok(());
        }
        let len = disk
            .map(|d| d.bytes().len())
            .ok_or_else(|| "in-flight sector transfer targets a drive with no disk mounted".to_string())?;
        let end = t
            .offset
            .checked_add(t.total)
            .ok_or_else(|| format!("Transfer.offset ({}) + Transfer.total ({}) overflows", t.offset, t.total))?;
        if end > len {
            return Err(format!(
                "Transfer.offset ({}) + Transfer.total ({}) = {end} exceeds the mounted disk's {len} bytes",
                t.offset, t.total
            ));
        }
        Ok(())
    }

    /// Advance the command state machine by `cycles` CPU cycles. `disk`/`side`
    /// are the currently-selected drive (per DSKREG) and its side select —
    /// needed for the not-found detection and for locating the next sector of
    /// a multiple-sector transfer.
    pub fn tick(&mut self, mut cycles: u32, mut disk: Option<&mut JVCDisk>, side: u8) {
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
}
