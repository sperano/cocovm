//! DriveWire 4 — an in-process implementation of the *server* side of the
//! DriveWire protocol: a byte-stream RPC that lets NitrOS-9 (or DECB, via
//! HDB-DOS) address disk images living on the host instead of real
//! hardware, one 256-byte sector at a time. This module is the protocol
//! engine only — framing, opcodes, checksums, and the [`DwImage`] backing
//! store; the Becker-port register wiring ($FF41/$FF42) that feeds bytes
//! into [`DwServer::data_write`] and reads them back out of
//! [`DwServer::data_read`] from the CPU bus is a separate, later task.
//!
//! Opcode set, packet layout, checksum algorithm, and error codes are cited
//! from the DriveWire 4 Java server (`DWProtocolHandler.java`) and
//! pyDriveWire, cross-checked against NitrOS-9's driver-side implementation
//! (`dwio.asm`, `rbdw.asm`, `dwcheck.asm`), per the verified spec this
//! module was built from.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use serde::{Deserialize, Serialize};

/// Fixed sector size for DriveWire images: a flat file with sector N at
/// byte offset `SECTOR_SIZE * N`, no header, no metadata (same convention
/// as [`crate::vhd`]).
pub const SECTOR_SIZE: usize = 256;

/// Number of mount slots exposed to the UI. The wire protocol's drive byte
/// is a full 8 bits, but only this many slots are backed by an image here.
pub const DRIVE_COUNT: usize = 4;

/// DriveWire status/error codes, wire values from `DWProtocolHandler.java`.
pub mod error {
    /// Operation completed successfully.
    pub const OK: u8 = 0x00;
    /// A transmitted checksum didn't match the receiver's own sum of the
    /// bytes involved (see [`super::opcode::READEX`]/[`super::opcode::WRITE`]
    /// families).
    pub const CRC: u8 = 0xF3;
    /// The host read failed: an I/O error, or the requested LSN lies at or
    /// beyond the end of the mounted image.
    pub const READ: u8 = 0xF4;
    /// The host write failed (I/O error).
    pub const WRITE: u8 = 0xF5;
    /// The addressed drive has no image mounted, or the drive index is out
    /// of range.
    pub const NOT_READY: u8 = 0xF6;
}

/// DriveWire opcodes, wire values from `DWProtocolHandler.java`.
pub mod opcode {
    /// No-op: single byte, no reply.
    pub const NOP: u8 = 0x00;
    /// Legacy client init handshake (distinct from [`DWINIT`]): single
    /// byte, no reply.
    pub const INIT: u8 = 0x49;
    /// Client terminate: single byte, no reply.
    pub const TERM: u8 = 0x54;
    /// Reset. Three wire values all mean the same thing
    /// (`DWProtocolHandler.java` accepts all three): single byte, no reply.
    pub const RESET1: u8 = 0xF8;
    pub const RESET2: u8 = 0xFE;
    pub const RESET3: u8 = 0xFF;
    /// Read the host wall clock: single-byte request, 6-byte reply.
    pub const TIME: u8 = 0x23;
    /// DriveWire 4 protocol handshake: 1-byte payload (client driver
    /// version, ignored), 1-byte reply ([`super::DW_PROTOCOL_VERSION`]).
    pub const DWINIT: u8 = 0x5A;
    /// Get device status: 2-byte payload (drive, statcode), no reply.
    pub const GETSTAT: u8 = 0x47;
    /// Set device status: 2-byte payload (drive, statcode), no reply.
    pub const SETSTAT: u8 = 0x53;
    /// Read a sector: 4-byte payload (drive, 24-bit big-endian LSN). Reply
    /// is a status byte, and — only if that status is [`super::error::OK`]
    /// — 256 data bytes plus a 2-byte big-endian checksum.
    pub const READ: u8 = 0x52;
    /// Same wire behaviour as [`READ`]; sent by the client to retry after a
    /// checksum failure.
    pub const REREAD: u8 = 0x72;
    /// Read a sector with a client-verified checksum: 4-byte payload; the
    /// server always sends 256 data bytes, then waits for the client's
    /// 2-byte checksum before replying with a single status byte.
    pub const READEX: u8 = 0xD2;
    /// Same wire behaviour as [`READEX`]; sent by the client to retry.
    pub const REREADEX: u8 = 0xF2;
    /// Write a sector: 4-byte payload, 256 data bytes, 2-byte big-endian
    /// checksum; reply is a single status byte.
    pub const WRITE: u8 = 0x57;
    /// Same wire behaviour as [`WRITE`]; sent by the client to retry.
    pub const REWRITE: u8 = 0x77;

    /// Virtual-serial-port family (`OP_SER*`): NitrOS-9's `scdwv`/`dwio`
    /// drivers use these to run DriveWire 4's telnet-over-virtual-serial
    /// feature. Wire values and payload shapes are from NitrOS-9's
    /// `defs/drivewire.d` (`OP_SERINIT`/`OP_SERTERM`/`OP_SERREAD`
    /// (`'C'` = $43)/`OP_SERREADM` (`'c'` = $63)/`OP_SERWRITE`
    /// (`'C'+128` = $C3)/`OP_SERGETSTAT` (`'D'` = $44)/`OP_SERSETSTAT`
    /// (`'D'+128` = $C4)), as quoted in
    /// `crates/coco-core/tests/drivewire_boot.rs`'s
    /// `nitros9_l2_boots_over_drivewire_to_shell_prompt` doc comment;
    /// reply shapes cross-checked against `scdwv.asm` and the DW4 Java
    /// server.
    ///
    /// Poll for pending virtual-serial input: no payload. The server here
    /// never has any (no real serial/telnet backing), so the reply is
    /// always exactly 2 bytes `0x00 0x00` ("idle, no data").
    pub const SERREAD: u8 = 0x43;
    /// Read multiple virtual-serial bytes: 2-byte payload (channel,
    /// count). Reply is `count` raw data bytes; unreachable from a real
    /// client here since [`SERREAD`] always reports idle, but still
    /// implemented so a client that sends it anyway stays in sync.
    pub const SERREADM: u8 = 0x63;
    /// Write to a virtual-serial channel: 2-byte payload (channel, data
    /// byte), no reply.
    pub const SERWRITE: u8 = 0xC3;
    /// Get virtual-serial channel status: 2-byte payload (channel,
    /// statcode), no reply.
    pub const SERGETSTAT: u8 = 0x44;
    /// Set virtual-serial channel status: 2-byte payload (channel,
    /// statcode), no reply — except when statcode is
    /// [`super::SS_COMST`], which is followed by
    /// [`super::COMST_PAYLOAD_LEN`] more raw bytes that must also be
    /// consumed (still no reply).
    pub const SERSETSTAT: u8 = 0xC4;
    /// Initialize a virtual-serial channel: 1-byte payload (channel), no
    /// reply.
    pub const SERINIT: u8 = 0x45;
    /// Terminate a virtual-serial channel: 1-byte payload (channel), no
    /// reply.
    pub const SERTERM: u8 = 0xC5;
    /// Fast-write to a virtual-serial channel, one opcode value per
    /// channel (channel number = opcode byte minus [`FASTWRITE_BASE`]):
    /// 1-byte payload (data byte), no reply.
    pub const FASTWRITE_BASE: u8 = 0x80;
    pub const FASTWRITE_LAST: u8 = 0x8F;
}

/// [`opcode::DWINIT`]'s reply byte: the DriveWire protocol version this
/// server implements. NitrOS-9 accepts only `0x04` or `0xFF` and aborts the
/// connection otherwise.
const DW_PROTOCOL_VERSION: u8 = 0x04;

/// Byte length of a READ/REREAD/READEX/REREADEX/WRITE/REWRITE header: drive
/// number (1 byte) + 24-bit big-endian LSN (3 bytes).
const HEADER_LEN: usize = 4;

/// Byte length of GETSTAT/SETSTAT's payload: drive number, statcode.
const STAT_PAYLOAD_LEN: u8 = 2;

/// [`opcode::SERSETSTAT`]'s `SS.ComSt` statcode value: NitrOS-9's SCF
/// device driver "set communication status" call, per `scdwv.asm`'s
/// `OPTCNT` (the `PD.OPT` option table referenced in `scf.d`).
const SS_COMST: u8 = 0x28;

/// Byte length of the SCF device-descriptor option table that
/// [`opcode::SERSETSTAT`] must consume (but not dispatch as opcodes)
/// after an [`SS_COMST`] statcode, per `scdwv.asm`'s `OPTCNT` and DW4's
/// `comRead(26)`.
const COMST_PAYLOAD_LEN: usize = 26;

/// Byte length of a WRITE/REWRITE request body after the opcode: header +
/// 256 sector data bytes + 2-byte checksum.
const WRITE_BODY_LEN: usize = HEADER_LEN + SECTOR_SIZE + 2;

/// HDB-DOS flat addressing: sectors per virtual disk (35 tracks × 18
/// sectors/track — a standard DECB `.dsk` geometry). In HDB-DOS mode
/// ([`DwServer::set_hdbdos_mode`]) the wire drive byte is ignored;
/// `drive = lsn / HDBDOS_SECTORS_PER_DISK`, local
/// `lsn = lsn % HDBDOS_SECTORS_PER_DISK`.
pub const HDBDOS_SECTORS_PER_DISK: u64 = 630;

/// CoCo 3 maximum CPU clock (double-speed GIME POKE), in Hz. Duplicated
/// from the private `CPU_HZ`/speed-doubling logic in `lib.rs` (this module
/// must stay bus/host-free, so it can't import that) purely to document the
/// derivation of [`TRANSACTION_TIMEOUT_CYCLES`] below.
const MAX_CPU_HZ: f64 = 1_789_772.5;

/// DriveWire transaction timeout, in seconds: 250 ms of CPU time with no
/// byte from the client aborts an in-progress transaction.
const TRANSACTION_TIMEOUT_SECONDS: f64 = 0.25;

/// [`TRANSACTION_TIMEOUT_SECONDS`] expressed in CPU cycles at
/// [`MAX_CPU_HZ`] (`1_789_772.5 * 0.25 = 447_443.125`, truncated to whole
/// cycles): if the server is mid-transaction (awaiting payload or checksum
/// bytes) and this many cycles pass with no byte arriving, the transaction
/// is abandoned and the next byte fed in is parsed as a fresh opcode
/// instead of more of the old transaction's payload.
pub const TRANSACTION_TIMEOUT_CYCLES: u64 = (MAX_CPU_HZ * TRANSACTION_TIMEOUT_SECONDS) as u64;

/// [`opcode::TIME`]'s reply encodes the year as an offset from 1900 (BASIC
/// convention), truncated to a `u8` (wrapping past 2155 — no CoCo software
/// written against this protocol needs to handle that).
const TIME_REPLY_YEAR_BASE: u16 = 1900;

/// Becker-port status bit meaning "at least one reply byte is ready for the
/// client to read" ([`DwServer::status_read`]). The Becker-port register
/// wiring itself ($FF41/$FF42) is out of scope for this module — this
/// constant only names the bit value this in-process server reports.
const STATUS_DATA_AVAILABLE: u8 = 0x02;

/// A DriveWire backing image: either an in-memory buffer (tests — small,
/// cheap to construct and assert against) or a real file, accessed by
/// seeking rather than loaded whole. Mirrors [`crate::vhd::VhdImage`] with
/// one deliberate difference: a read whose sector lies fully or partly
/// beyond the image's current length is an *error* here (DriveWire has no
/// "sparse image" semantics — a read past the end means the client asked
/// for an LSN the image doesn't have), whereas a write at or beyond the end
/// silently extends the image (so a fresh, empty image file can become a
/// valid disk just by formatting it — DECB `FORMAT`/NitrOS-9 `format` write
/// every sector of a new volume in ascending LSN order).
pub enum DwImage {
    Memory(Vec<u8>),
    File(File),
}

impl DwImage {
    /// Current length of the backing image in bytes.
    fn len(&self) -> io::Result<u64> {
        match self {
            DwImage::Memory(bytes) => Ok(bytes.len() as u64),
            DwImage::File(file) => Ok(file.metadata()?.len()),
        }
    }

    /// Read exactly `buf.len()` bytes starting at `offset`. Returns an
    /// error (mapped by the caller to [`error::READ`]) if `offset..offset +
    /// buf.len()` runs past the image's current length, or on a genuine
    /// host I/O error.
    pub(crate) fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let len = self.len()?;
        if offset.saturating_add(buf.len() as u64) > len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "DriveWire read past end of image",
            ));
        }
        match self {
            DwImage::Memory(bytes) => {
                let start = offset as usize;
                buf.copy_from_slice(&bytes[start..start + buf.len()]);
                Ok(())
            }
            DwImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(buf)
            }
        }
    }

    /// Write `buf` at `offset`, growing the image if `offset + buf.len()`
    /// exceeds the current length (for `Memory`, resizing zero-fills any
    /// newly created gap before `offset`; for `File`, seeking past the
    /// current end and writing extends it the same way a real file does).
    pub(crate) fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        match self {
            DwImage::Memory(bytes) => {
                let end = offset as usize + buf.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[offset as usize..end].copy_from_slice(buf);
                Ok(())
            }
            DwImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(buf)
            }
        }
    }

    /// The image's raw bytes, for inspection — only meaningful for the
    /// in-memory variant (tests construct one, mount it, then read this
    /// back to check what a command wrote); `None` for a file-backed image.
    pub fn as_memory(&self) -> Option<&[u8]> {
        match self {
            DwImage::Memory(bytes) => Some(bytes),
            DwImage::File(_) => None,
        }
    }
}

/// A wall-clock timestamp for [`opcode::TIME`] replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DwTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// A source of wall-clock time for [`opcode::TIME`], injected by the
/// frontend — `coco-core` has no host clock access of its own.
pub type DwClock = Box<dyn FnMut() -> DwTime + Send>;

/// [`DwServer`]'s default clock (before [`DwServer::set_clock`] is called):
/// an arbitrary but deterministic fixed date, since `coco-core` must stay
/// host-free. The frontend crate injects the real wall clock.
const DEFAULT_CLOCK_YEAR: u16 = 1990;
const DEFAULT_CLOCK_MONTH: u8 = 1;
const DEFAULT_CLOCK_DAY: u8 = 1;
const DEFAULT_CLOCK_HOUR: u8 = 0;
const DEFAULT_CLOCK_MINUTE: u8 = 0;
const DEFAULT_CLOCK_SECOND: u8 = 0;

fn default_clock() -> DwTime {
    DwTime {
        year: DEFAULT_CLOCK_YEAR,
        month: DEFAULT_CLOCK_MONTH,
        day: DEFAULT_CLOCK_DAY,
        hour: DEFAULT_CLOCK_HOUR,
        minute: DEFAULT_CLOCK_MINUTE,
        second: DEFAULT_CLOCK_SECOND,
    }
}

/// `#[serde(default = "...")]` for [`DwServer::clock`]: matches
/// [`DwServer::new`]'s own default (a closure has no serializable shape, so
/// this is what a restored server falls back to until the frontend calls
/// [`DwServer::set_clock`] again — `docs/plan-save-states.md`).
fn default_dw_clock() -> DwClock {
    Box::new(default_clock)
}

/// Plain 16-bit sum of a 256-byte sector's bytes. Despite [`error::CRC`]'s
/// name, DriveWire's "checksum" is this trivial running sum, not a CRC: all
/// bytes 0xFF sums to `256 * 255 = 65_280`, which fits in a `u16` with no
/// wraparound possible, so this never needs `wrapping_add`.
fn checksum_of(sector: &[u8]) -> u16 {
    sector.iter().map(|&b| u16::from(b)).sum()
}

/// One in-progress DriveWire transaction. An opcode byte is only ever
/// parsed from [`State::Idle`] — a byte arriving mid-transaction is always
/// consumed as more of that transaction's payload, never reinterpreted as a
/// fresh opcode (that's what [`DwServer::data_write`]'s timeout check is
/// for). This is also why [`opcode::RESET`](opcode) needs no special
/// handling beyond being a normal opcode: by the time an opcode byte is
/// parsed, whatever transaction there was has already ended (successfully,
/// on error, or via timeout).
#[derive(Serialize, Deserialize)]
enum State {
    Idle,
    /// [`opcode::DWINIT`] sent; awaiting the client's 1-byte driver version
    /// (ignored) before replying with [`DW_PROTOCOL_VERSION`].
    AwaitDwInitVersion,
    /// A "consume `remaining` more bytes, then send no reply" transaction.
    /// This wire shape is shared by several unrelated opcodes:
    /// [`opcode::GETSTAT`]/[`opcode::SETSTAT`] (drive, statcode),
    /// [`opcode::SERGETSTAT`] (channel, statcode),
    /// [`opcode::SERWRITE`] (channel, data byte),
    /// [`opcode::SERINIT`]/[`opcode::SERTERM`] (channel), the
    /// [`opcode::FASTWRITE_BASE`]..=[`opcode::FASTWRITE_LAST`] family
    /// (data byte), and the trailing [`COMST_PAYLOAD_LEN`]-byte option
    /// table after an [`SS_COMST`] [`opcode::SERSETSTAT`] (see
    /// [`State::AwaitSerSetStat`]) — hence one shared variant rather than
    /// a sibling per opcode.
    AwaitDiscard {
        remaining: u8,
    },
    /// [`opcode::SERREADM`] sent; awaiting its 2-byte payload (channel,
    /// count). Once both bytes arrive, replies with `count` zero bytes
    /// (this server never has real virtual-serial data pending, and
    /// [`opcode::SERREAD`] always reports "idle" first, so a real client
    /// never actually reaches this opcode — but the wire shape is still
    /// honored for one that does).
    AwaitSerReadM {
        buf: Vec<u8>,
    },
    /// [`opcode::SERSETSTAT`] sent; awaiting its 2-byte payload (channel,
    /// statcode). Once both bytes arrive, branches on the statcode: if it
    /// is [`SS_COMST`], transitions to [`State::AwaitDiscard`] with
    /// [`COMST_PAYLOAD_LEN`] more bytes to consume (the SCF
    /// device-descriptor option table that always accompanies `SS.ComSt`
    /// on the wire); otherwise the transaction is already complete (no
    /// reply either way).
    AwaitSerSetStat {
        buf: Vec<u8>,
    },
    /// A READ-family opcode sent; awaiting the rest of the 4-byte header.
    /// `ex` distinguishes the READEX/REREADEX group (always sends 256 bytes
    /// and awaits a client checksum) from READ/REREAD (single status byte,
    /// or status + data + checksum on success).
    AwaitReadHeader {
        ex: bool,
        buf: Vec<u8>,
    },
    /// A READEX-family read's 256 data bytes have been sent; awaiting the
    /// client's 2-byte checksum. `expected` is this server's own checksum
    /// of the bytes it sent; `pending_error` is the read's outcome
    /// ([`error::OK`] or the error code the plain READ path would have sent
    /// as its status byte) to reply with if the client's checksum matches —
    /// a checksum mismatch overrides it with [`error::CRC`] instead.
    AwaitReadExChecksum {
        expected: u16,
        pending_error: u8,
        buf: Vec<u8>,
    },
    /// A WRITE-family opcode sent; awaiting the rest of the 262-byte body
    /// (header + 256 data bytes + 2-byte checksum).
    AwaitWriteBody {
        buf: Vec<u8>,
    },
}

/// The DriveWire server: mounted images, the protocol state machine, and
/// the reply FIFO the Becker-port bus wiring drains from.
#[derive(Serialize, Deserialize)]
pub struct DwServer {
    /// Skipped: each mounted image can hold an open host `File` handle —
    /// remounted by path on restore via [`DwServer::reattach`]
    /// (`docs/plan-save-states.md`).
    #[serde(skip)]
    drives: [Option<DwImage>; DRIVE_COUNT],
    /// Set on a successful [`opcode::WRITE`]/[`opcode::REWRITE`]; cleared by
    /// [`DwServer::mount`]/[`DwServer::eject`].
    dirty: [bool; DRIVE_COUNT],
    reply: VecDeque<u8>,
    state: State,
    hdbdos: bool,
    /// Skipped: a closure has no serializable shape. Restored to
    /// [`default_dw_clock`] until the frontend calls [`DwServer::set_clock`]
    /// again (`docs/plan-save-states.md`).
    #[serde(skip, default = "default_dw_clock")]
    clock: DwClock,
    sectors_read: u64,
    sectors_written: u64,
    unknown_opcodes: u64,
    /// Count of virtual-serial-port opcodes handled (see [`opcode`]'s
    /// `OP_SER*`/[`opcode::FASTWRITE_BASE`] family), incremented once per
    /// top-level operation dispatched — not per byte consumed.
    vserial_ops: u64,
    /// Cycle stamp of the last byte fed via [`DwServer::data_write`], for
    /// the transaction timeout. `None` before the first byte ever arrives.
    last_byte_cycle: Option<u64>,
}

impl DwServer {
    pub fn new() -> Self {
        Self {
            drives: std::array::from_fn(|_| None),
            dirty: [false; DRIVE_COUNT],
            reply: VecDeque::new(),
            state: State::Idle,
            hdbdos: false,
            clock: Box::new(default_clock),
            sectors_read: 0,
            sectors_written: 0,
            unknown_opcodes: 0,
            vserial_ops: 0,
            last_byte_cycle: None,
        }
    }

    /// Mount `image` in `drive`, replacing anything already there and
    /// clearing its dirty flag.
    pub fn mount(&mut self, drive: usize, image: DwImage) {
        self.drives[drive] = Some(image);
        self.dirty[drive] = false;
    }

    /// Unmount `drive`'s image, if any, and clear its dirty flag.
    pub fn eject(&mut self, drive: usize) {
        self.drives[drive] = None;
        self.dirty[drive] = false;
    }

    /// Restore-path-only: re-inject a mounted image after a snapshot
    /// restore, WITHOUT clearing `dirty[drive]` (unlike [`DwServer::mount`])
    /// — the restored dirty flag is itself real machine state, not reset by
    /// remounting the same image the snapshot already had open
    /// (`docs/plan-save-states.md`).
    pub fn reattach(&mut self, drive: usize, image: DwImage) {
        self.drives[drive] = Some(image);
    }

    pub fn is_mounted(&self, drive: usize) -> bool {
        self.drives[drive].is_some()
    }

    /// The image mounted in `drive`, if any — mainly for tests (see
    /// [`DwImage::as_memory`]).
    pub fn image(&self, drive: usize) -> Option<&DwImage> {
        self.drives[drive].as_ref()
    }

    /// Enable/disable HDB-DOS flat addressing (see
    /// [`HDBDOS_SECTORS_PER_DISK`]). Off by default: plain DriveWire, where
    /// the wire drive byte selects the mount slot directly.
    pub fn set_hdbdos_mode(&mut self, enabled: bool) {
        self.hdbdos = enabled;
    }

    pub fn hdbdos_mode(&self) -> bool {
        self.hdbdos
    }

    pub fn dirty(&self, drive: usize) -> bool {
        self.dirty[drive]
    }

    pub fn sectors_read(&self) -> u64 {
        self.sectors_read
    }

    pub fn sectors_written(&self) -> u64 {
        self.sectors_written
    }

    pub fn unknown_opcodes(&self) -> u64 {
        self.unknown_opcodes
    }

    pub fn vserial_ops(&self) -> u64 {
        self.vserial_ops
    }

    /// Inject a wall clock for [`opcode::TIME`] replies (the frontend's
    /// job — `coco-core` itself only ever uses [`default_clock`]).
    pub fn set_clock(&mut self, clock: DwClock) {
        self.clock = clock;
    }

    /// Becker-port status register read: [`STATUS_DATA_AVAILABLE`] set
    /// whenever at least one reply byte is queued, `0` otherwise.
    /// Non-destructive, unlike [`DwServer::data_read`].
    pub fn status_read(&self) -> u8 {
        if self.reply.is_empty() {
            0
        } else {
            STATUS_DATA_AVAILABLE
        }
    }

    /// Becker-port data register read: pop and return the next queued reply
    /// byte, or `0x00` if the FIFO is empty.
    pub fn data_read(&mut self) -> u8 {
        self.reply.pop_front().unwrap_or(0)
    }

    /// Becker-port data register write: feed one byte from the client into
    /// the protocol state machine. `cycle` is a monotonically increasing
    /// CPU cycle count, used only to detect a stalled transaction (see
    /// [`TRANSACTION_TIMEOUT_CYCLES`]); compared against the previous
    /// byte's cycle with `wrapping_sub` so a 64-bit wraparound (unreachable
    /// in practice, but cheap to get right) can't misfire.
    pub fn data_write(&mut self, byte: u8, cycle: u64) {
        if let Some(prev) = self.last_byte_cycle {
            let idle = matches!(self.state, State::Idle);
            if !idle && cycle.wrapping_sub(prev) > TRANSACTION_TIMEOUT_CYCLES {
                self.state = State::Idle;
            }
        }
        self.last_byte_cycle = Some(cycle);
        self.feed(byte);
    }

    /// Route one byte through the state machine.
    fn feed(&mut self, byte: u8) {
        let state = std::mem::replace(&mut self.state, State::Idle);
        match state {
            State::Idle => self.handle_opcode(byte),
            State::AwaitDwInitVersion => {
                // The client driver version byte itself is ignored.
                self.reply.push_back(DW_PROTOCOL_VERSION);
            }
            State::AwaitDiscard { remaining } => {
                if remaining > 1 {
                    self.state = State::AwaitDiscard {
                        remaining: remaining - 1,
                    };
                }
            }
            State::AwaitSerReadM { mut buf } => {
                buf.push(byte);
                if buf.len() == 2 {
                    let count = buf[1];
                    self.reply.extend(std::iter::repeat_n(0u8, count as usize));
                } else {
                    self.state = State::AwaitSerReadM { buf };
                }
            }
            State::AwaitSerSetStat { mut buf } => {
                buf.push(byte);
                if buf.len() == 2 {
                    let statcode = buf[1];
                    if statcode == SS_COMST {
                        self.state = State::AwaitDiscard {
                            remaining: COMST_PAYLOAD_LEN as u8,
                        };
                    }
                    // Otherwise the transaction is complete: no reply,
                    // state stays Idle (already set at the top of `feed`).
                } else {
                    self.state = State::AwaitSerSetStat { buf };
                }
            }
            State::AwaitReadHeader { ex, mut buf } => {
                buf.push(byte);
                if buf.len() == HEADER_LEN {
                    self.execute_read(ex, &buf);
                } else {
                    self.state = State::AwaitReadHeader { ex, buf };
                }
            }
            State::AwaitReadExChecksum {
                expected,
                pending_error,
                mut buf,
            } => {
                buf.push(byte);
                if buf.len() == 2 {
                    let client_sum = (u16::from(buf[0]) << 8) | u16::from(buf[1]);
                    let status = if client_sum != expected {
                        error::CRC
                    } else {
                        pending_error
                    };
                    self.reply.push_back(status);
                } else {
                    self.state = State::AwaitReadExChecksum {
                        expected,
                        pending_error,
                        buf,
                    };
                }
            }
            State::AwaitWriteBody { mut buf } => {
                buf.push(byte);
                if buf.len() == WRITE_BODY_LEN {
                    self.execute_write(&buf);
                } else {
                    self.state = State::AwaitWriteBody { buf };
                }
            }
        }
    }

    /// Dispatch a byte parsed as a fresh opcode (always called with
    /// `self.state == State::Idle`).
    fn handle_opcode(&mut self, op: u8) {
        match op {
            opcode::NOP
            | opcode::INIT
            | opcode::TERM
            | opcode::RESET1
            | opcode::RESET2
            | opcode::RESET3 => {
                // Single byte, no reply, no state change.
            }
            opcode::TIME => {
                let t = (self.clock)();
                self.reply
                    .push_back(t.year.wrapping_sub(TIME_REPLY_YEAR_BASE) as u8);
                self.reply.push_back(t.month);
                self.reply.push_back(t.day);
                self.reply.push_back(t.hour);
                self.reply.push_back(t.minute);
                self.reply.push_back(t.second);
            }
            opcode::DWINIT => {
                self.state = State::AwaitDwInitVersion;
            }
            opcode::GETSTAT | opcode::SETSTAT => {
                self.state = State::AwaitDiscard {
                    remaining: STAT_PAYLOAD_LEN,
                };
            }
            opcode::SERREAD => {
                self.vserial_ops += 1;
                // Always "idle, no data" — this server never has real
                // virtual-serial input pending.
                self.reply.push_back(0x00);
                self.reply.push_back(0x00);
            }
            opcode::SERREADM => {
                self.vserial_ops += 1;
                self.state = State::AwaitSerReadM {
                    buf: Vec::with_capacity(2),
                };
            }
            opcode::SERWRITE | opcode::SERGETSTAT => {
                self.vserial_ops += 1;
                self.state = State::AwaitDiscard { remaining: 2 };
            }
            opcode::SERSETSTAT => {
                self.vserial_ops += 1;
                self.state = State::AwaitSerSetStat {
                    buf: Vec::with_capacity(2),
                };
            }
            opcode::SERINIT | opcode::SERTERM => {
                self.vserial_ops += 1;
                self.state = State::AwaitDiscard { remaining: 1 };
            }
            opcode::FASTWRITE_BASE..=opcode::FASTWRITE_LAST => {
                self.vserial_ops += 1;
                self.state = State::AwaitDiscard { remaining: 1 };
            }
            opcode::READ | opcode::REREAD => {
                self.state = State::AwaitReadHeader {
                    ex: false,
                    buf: Vec::with_capacity(HEADER_LEN),
                };
            }
            opcode::READEX | opcode::REREADEX => {
                self.state = State::AwaitReadHeader {
                    ex: true,
                    buf: Vec::with_capacity(HEADER_LEN),
                };
            }
            opcode::WRITE | opcode::REWRITE => {
                self.state = State::AwaitWriteBody {
                    buf: Vec::with_capacity(WRITE_BODY_LEN),
                };
            }
            _ => {
                self.unknown_opcodes += 1;
            }
        }
    }

    /// Decode a READ/READEX/WRITE header: byte 0 is the drive number, bytes
    /// 1..4 are the 24-bit big-endian LSN. In HDB-DOS mode the wire drive
    /// byte is ignored and both drive and local LSN are derived from the
    /// LSN alone (see [`HDBDOS_SECTORS_PER_DISK`]).
    fn decode_header(&self, header: &[u8]) -> (usize, u64) {
        let wire_drive = header[0] as usize;
        let lsn = (u64::from(header[1]) << 16) | (u64::from(header[2]) << 8) | u64::from(header[3]);
        if self.hdbdos {
            let drive = (lsn / HDBDOS_SECTORS_PER_DISK) as usize;
            let local_lsn = lsn % HDBDOS_SECTORS_PER_DISK;
            (drive, local_lsn)
        } else {
            (wire_drive, lsn)
        }
    }

    /// Read the 256-byte sector at `lsn` from `drive`'s mounted image.
    /// [`error::NOT_READY`] if the drive index is out of range or
    /// unmounted; [`error::READ`] if the LSN is beyond the image's current
    /// length or the host read failed.
    fn read_sector(&mut self, drive: usize, lsn: u64) -> Result<[u8; SECTOR_SIZE], u8> {
        let image = self
            .drives
            .get_mut(drive)
            .and_then(|d| d.as_mut())
            .ok_or(error::NOT_READY)?;
        let mut sector = [0u8; SECTOR_SIZE];
        image
            .read_at(lsn * SECTOR_SIZE as u64, &mut sector)
            .map_err(|_| error::READ)?;
        Ok(sector)
    }

    /// Write `sector` to `drive` at `lsn`. [`error::NOT_READY`] if the
    /// drive is unmounted/out of range, [`error::WRITE`] on a host I/O
    /// error, else [`error::OK`] (which also marks the drive dirty and
    /// bumps [`DwServer::sectors_written`]).
    fn write_sector(&mut self, drive: usize, lsn: u64, sector: &[u8]) -> u8 {
        let Some(image) = self.drives.get_mut(drive).and_then(|d| d.as_mut()) else {
            return error::NOT_READY;
        };
        match image.write_at(lsn * SECTOR_SIZE as u64, sector) {
            Ok(()) => {
                self.sectors_written += 1;
                if let Some(dirty) = self.dirty.get_mut(drive) {
                    *dirty = true;
                }
                error::OK
            }
            Err(_) => error::WRITE,
        }
    }

    /// Execute a completed READ/REREAD/READEX/REREADEX header: `ex`
    /// selects the READEX-family wire behaviour (see [`State`]).
    fn execute_read(&mut self, ex: bool, header: &[u8]) {
        let (drive, lsn) = self.decode_header(header);
        if ex {
            let (data, pending_error) = match self.read_sector(drive, lsn) {
                Ok(sector) => {
                    self.sectors_read += 1;
                    (sector, error::OK)
                }
                Err(status) => ([0u8; SECTOR_SIZE], status),
            };
            let expected = checksum_of(&data);
            self.reply.extend(data);
            self.state = State::AwaitReadExChecksum {
                expected,
                pending_error,
                buf: Vec::with_capacity(2),
            };
        } else {
            match self.read_sector(drive, lsn) {
                Ok(sector) => {
                    self.sectors_read += 1;
                    self.reply.push_back(error::OK);
                    let checksum = checksum_of(&sector);
                    self.reply.extend(sector);
                    self.reply.push_back((checksum >> 8) as u8);
                    self.reply.push_back((checksum & 0xFF) as u8);
                }
                Err(status) => self.reply.push_back(status),
            }
        }
    }

    /// Execute a completed WRITE/REWRITE body (header + 256 data bytes +
    /// 2-byte checksum, [`WRITE_BODY_LEN`] bytes total).
    fn execute_write(&mut self, body: &[u8]) {
        let header = &body[0..HEADER_LEN];
        let sector = &body[HEADER_LEN..HEADER_LEN + SECTOR_SIZE];
        let received = (u16::from(body[HEADER_LEN + SECTOR_SIZE]) << 8)
            | u16::from(body[HEADER_LEN + SECTOR_SIZE + 1]);
        if received != checksum_of(sector) {
            self.reply.push_back(error::CRC);
            return;
        }
        let (drive, lsn) = self.decode_header(header);
        let status = self.write_sector(drive, lsn, sector);
        self.reply.push_back(status);
    }
}

impl Default for DwServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recognizable, non-repeating 256-byte pattern: byte `i` has value
    /// `i as u8`.
    fn pattern_sector() -> [u8; SECTOR_SIZE] {
        let mut s = [0u8; SECTOR_SIZE];
        for (i, b) in s.iter_mut().enumerate() {
            *b = i as u8;
        }
        s
    }

    fn lsn_bytes(lsn: u32) -> [u8; 3] {
        [(lsn >> 16) as u8, (lsn >> 8) as u8, lsn as u8]
    }

    /// Feed every byte of `bytes` into `server`, each at its own
    /// (small, strictly increasing) fake cycle count, then drain and return
    /// whatever reply bytes are queued afterward.
    fn feed_and_drain(server: &mut DwServer, bytes: &[u8]) -> Vec<u8> {
        feed_at(server, bytes, 0);
        drain(server)
    }

    /// Like [`feed_and_drain`] but the first byte lands at cycle `start`
    /// (subsequent bytes increment by 1), without draining afterward.
    fn feed_at(server: &mut DwServer, bytes: &[u8], start: u64) {
        for (i, &b) in bytes.iter().enumerate() {
            server.data_write(b, start + i as u64);
        }
    }

    fn drain(server: &mut DwServer) -> Vec<u8> {
        let mut out = Vec::new();
        while server.status_read() != 0 {
            out.push(server.data_read());
        }
        out
    }

    #[test]
    fn checksum_vectors() {
        assert_eq!(checksum_of(&[0u8; SECTOR_SIZE]), 0);
        assert_eq!(checksum_of(&[0xFFu8; SECTOR_SIZE]), 65_280);
        assert_eq!(checksum_of(&pattern_sector()), 32_640);
    }

    #[test]
    fn dwinit_replies_protocol_version() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x03]);
        assert_eq!(reply, vec![0x04]);
    }

    #[test]
    fn time_uses_injected_clock() {
        let mut server = DwServer::new();
        server.set_clock(Box::new(|| DwTime {
            year: 2024,
            month: 6,
            day: 15,
            hour: 12,
            minute: 30,
            second: 45,
        }));
        let reply = feed_and_drain(&mut server, &[opcode::TIME]);
        assert_eq!(reply, vec![124, 6, 15, 12, 30, 45]);
    }

    #[test]
    fn time_default_clock_is_fixed_date() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(&mut server, &[opcode::TIME]);
        assert_eq!(reply, vec![90, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn read_success() {
        let mut server = DwServer::new();
        let mut image = vec![0u8; SECTOR_SIZE * 2];
        image[SECTOR_SIZE..].copy_from_slice(&pattern_sector());
        server.mount(0, DwImage::Memory(image));

        let mut req = vec![opcode::READ, 0];
        req.extend(lsn_bytes(1));
        let reply = feed_and_drain(&mut server, &req);

        assert_eq!(reply[0], error::OK);
        assert_eq!(&reply[1..1 + SECTOR_SIZE], &pattern_sector());
        let checksum = checksum_of(&pattern_sector());
        assert_eq!(reply[1 + SECTOR_SIZE], (checksum >> 8) as u8);
        assert_eq!(reply[2 + SECTOR_SIZE], (checksum & 0xFF) as u8);
        assert_eq!(server.sectors_read(), 1);
    }

    #[test]
    fn read_unmounted_drive() {
        let mut server = DwServer::new();
        let mut req = vec![opcode::READ, 0];
        req.extend(lsn_bytes(0));
        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::NOT_READY]);
    }

    #[test]
    fn read_lsn_past_end() {
        let mut server = DwServer::new();
        server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));
        let mut req = vec![opcode::READ, 0];
        req.extend(lsn_bytes(1));
        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::READ]);
    }

    #[test]
    fn readex_round_trip_and_retry() {
        let mut server = DwServer::new();
        let mut image = vec![0u8; SECTOR_SIZE];
        image.copy_from_slice(&pattern_sector());
        server.mount(0, DwImage::Memory(image));

        let mut req = vec![opcode::READEX, 0];
        req.extend(lsn_bytes(0));
        feed_at(&mut server, &req, 0);
        let data = drain(&mut server);
        assert_eq!(data.len(), SECTOR_SIZE);
        assert_eq!(data, pattern_sector());

        let checksum = checksum_of(&data);
        feed_at(
            &mut server,
            &[(checksum >> 8) as u8, (checksum & 0xFF) as u8],
            100,
        );
        assert_eq!(drain(&mut server), vec![error::OK]);

        // Wrong checksum -> CRC error.
        feed_at(&mut server, &req, 200);
        let _ = drain(&mut server);
        feed_at(&mut server, &[0x00, 0x00], 300);
        assert_eq!(drain(&mut server), vec![error::CRC]);

        // REREADEX retries successfully.
        let mut retry = vec![opcode::REREADEX, 0];
        retry.extend(lsn_bytes(0));
        feed_at(&mut server, &retry, 400);
        let data = drain(&mut server);
        let checksum = checksum_of(&data);
        feed_at(
            &mut server,
            &[(checksum >> 8) as u8, (checksum & 0xFF) as u8],
            500,
        );
        assert_eq!(drain(&mut server), vec![error::OK]);
    }

    #[test]
    fn readex_unmounted_drive_sends_zeros_with_pending_error() {
        let mut server = DwServer::new();
        let mut req = vec![opcode::READEX, 0];
        req.extend(lsn_bytes(0));
        feed_at(&mut server, &req, 0);
        let data = drain(&mut server);
        assert_eq!(data, vec![0u8; SECTOR_SIZE]);

        // Matching (zero-sum) checksum -> pending NOT_READY error preserved.
        feed_at(&mut server, &[0x00, 0x00], 100);
        assert_eq!(drain(&mut server), vec![error::NOT_READY]);

        // Wrong checksum overrides with CRC even though the drive is unmounted.
        feed_at(&mut server, &req, 200);
        let _ = drain(&mut server);
        feed_at(&mut server, &[0xFF, 0xFF], 300);
        assert_eq!(drain(&mut server), vec![error::CRC]);
    }

    #[test]
    fn write_success_sets_dirty_and_persists() {
        let mut server = DwServer::new();
        server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

        let sector = pattern_sector();
        let checksum = checksum_of(&sector);
        let mut req = vec![opcode::WRITE, 0];
        req.extend(lsn_bytes(0));
        req.extend(sector);
        req.push((checksum >> 8) as u8);
        req.push((checksum & 0xFF) as u8);

        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::OK]);
        assert!(server.dirty(0));
        assert_eq!(server.image(0).unwrap().as_memory().unwrap(), &sector);
        assert_eq!(server.sectors_written(), 1);
    }

    #[test]
    fn write_bad_checksum_leaves_image_untouched() {
        let mut server = DwServer::new();
        server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

        let sector = pattern_sector();
        let mut req = vec![opcode::WRITE, 0];
        req.extend(lsn_bytes(0));
        req.extend(sector);
        req.push(0x00); // wrong checksum
        req.push(0x00);

        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::CRC]);
        assert!(!server.dirty(0));
        assert_eq!(
            server.image(0).unwrap().as_memory().unwrap(),
            &[0u8; SECTOR_SIZE]
        );

        // REWRITE with the correct checksum then succeeds.
        let checksum = checksum_of(&sector);
        let mut retry = vec![opcode::REWRITE, 0];
        retry.extend(lsn_bytes(0));
        retry.extend(sector);
        retry.push((checksum >> 8) as u8);
        retry.push((checksum & 0xFF) as u8);
        assert_eq!(feed_and_drain(&mut server, &retry), vec![error::OK]);
    }

    #[test]
    fn write_unmounted_drive_still_consumes_all_bytes() {
        let mut server = DwServer::new();
        let sector = pattern_sector();
        let checksum = checksum_of(&sector);
        let mut req = vec![opcode::WRITE, 0];
        req.extend(lsn_bytes(0));
        req.extend(sector);
        req.push((checksum >> 8) as u8);
        req.push((checksum & 0xFF) as u8);

        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::NOT_READY]);

        // Server is back in sync: next opcode parses cleanly.
        let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x00]);
        assert_eq!(reply, vec![0x04]);
    }

    #[test]
    fn write_past_end_extends_image() {
        let mut server = DwServer::new();
        server.mount(0, DwImage::Memory(vec![0u8; SECTOR_SIZE]));

        let sector = pattern_sector();
        let checksum = checksum_of(&sector);
        let mut req = vec![opcode::WRITE, 0];
        req.extend(lsn_bytes(1));
        req.extend(sector);
        req.push((checksum >> 8) as u8);
        req.push((checksum & 0xFF) as u8);

        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply, vec![error::OK]);
        let bytes = server.image(0).unwrap().as_memory().unwrap();
        assert_eq!(bytes.len(), SECTOR_SIZE * 2);
        assert_eq!(&bytes[SECTOR_SIZE..], &sector);
    }

    #[test]
    fn getstat_setstat_consume_exactly_two_bytes() {
        let mut server = DwServer::new();
        // GETSTAT + drive + statcode, then a fresh DWINIT — no leftover
        // reply from GETSTAT, and DWINIT parses cleanly right after.
        let reply = feed_and_drain(
            &mut server,
            &[opcode::GETSTAT, 0x00, 0x01, opcode::DWINIT, 0x00],
        );
        assert_eq!(reply, vec![0x04]);

        let reply = feed_and_drain(
            &mut server,
            &[opcode::SETSTAT, 0x00, 0x01, opcode::DWINIT, 0x00],
        );
        assert_eq!(reply, vec![0x04]);
    }

    #[test]
    fn serread_always_reports_idle() {
        let mut server = DwServer::new();
        for i in 0..3u64 {
            let reply = feed_and_drain(&mut server, &[opcode::SERREAD]);
            assert_eq!(reply, vec![0x00, 0x00], "iteration {i}");
        }
        assert_eq!(server.vserial_ops(), 3);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn serinit_serterm_sergetstat_consume_bytes_with_no_reply() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(
            &mut server,
            &[
                opcode::SERINIT,
                0x00, // channel
                opcode::SERTERM,
                0x00, // channel
                opcode::SERGETSTAT,
                0x00, // channel
                0x01, // statcode
                opcode::DWINIT,
                0x03, // driver version, ignored
            ],
        );
        // Only DWINIT produces a reply: proves the state machine is back
        // in sync, not desynced by any of the preceding SER* opcodes.
        assert_eq!(reply, vec![0x04]);
        assert_eq!(server.vserial_ops(), 3);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn sersetstat_non_comst_consumes_two_bytes_only() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(
            &mut server,
            &[
                opcode::SERSETSTAT,
                0x00, // channel
                0x00, // statcode, not SS_COMST
                opcode::DWINIT,
                0x00,
            ],
        );
        assert_eq!(reply, vec![0x04]);
        assert_eq!(server.vserial_ops(), 1);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn sersetstat_comst_consumes_payload_without_dispatching_it_as_opcodes() {
        let mut server = DwServer::new();
        let mut req = vec![
            opcode::SERSETSTAT,
            0x00,     // channel
            SS_COMST, // statcode
        ];
        // One of the 26 payload bytes deliberately equals a real opcode
        // value (DWINIT) to prove it is consumed as raw payload, not
        // dispatched — if it were misparsed as an opcode, a stray 0x04
        // reply would appear before the real DWINIT below.
        let mut payload = vec![0u8; COMST_PAYLOAD_LEN];
        payload[10] = opcode::DWINIT;
        req.extend(payload);
        feed_at(&mut server, &req, 0);
        assert!(
            drain(&mut server).is_empty(),
            "SERSETSTAT + its ComSt payload must produce no reply"
        );

        let reply = feed_and_drain(&mut server, &[opcode::DWINIT, 0x00]);
        assert_eq!(reply, vec![0x04]);
        assert_eq!(server.vserial_ops(), 1);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn fastwrite_and_serwrite_consume_bytes_with_no_reply() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(
            &mut server,
            &[
                opcode::FASTWRITE_BASE,
                0xAB, // data byte, channel 0
                opcode::FASTWRITE_LAST,
                0xCD, // data byte, channel 15
                opcode::SERWRITE,
                0x00, // channel
                0xEF, // data byte
                opcode::DWINIT,
                0x00,
            ],
        );
        assert_eq!(reply, vec![0x04]);
        assert_eq!(server.vserial_ops(), 3);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn serreadm_replies_with_count_zero_bytes() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(&mut server, &[opcode::SERREADM, 0x00, 0x05]);
        assert_eq!(reply, vec![0u8; 5]);
        assert_eq!(server.vserial_ops(), 1);
        assert_eq!(server.unknown_opcodes(), 0);
    }

    #[test]
    fn unknown_opcode_is_silently_skipped() {
        let mut server = DwServer::new();
        let reply = feed_and_drain(&mut server, &[0xAB, opcode::DWINIT, 0x00]);
        assert_eq!(reply, vec![0x04]);
        assert_eq!(server.unknown_opcodes(), 1);
    }

    #[test]
    fn stalled_transaction_times_out() {
        let mut server = DwServer::new();
        // Start a READ but only send 2 of its 4 header bytes.
        feed_at(&mut server, &[opcode::READ, 0x00], 0);
        assert!(drain(&mut server).is_empty());

        // Next byte arrives well past the timeout: treated as a fresh
        // opcode (DWINIT) instead of header byte 3.
        let timeout_cycle = 1 + TRANSACTION_TIMEOUT_CYCLES + 1;
        feed_at(&mut server, &[opcode::DWINIT], timeout_cycle);
        feed_at(&mut server, &[0x00], timeout_cycle + 1);
        assert_eq!(drain(&mut server), vec![0x04]);
    }

    #[test]
    fn hdbdos_mode_remaps_drive_and_lsn() {
        let mut server = DwServer::new();
        server.set_hdbdos_mode(true);
        server.mount(0, DwImage::Memory(vec![0xAAu8; SECTOR_SIZE]));
        server.mount(1, DwImage::Memory(pattern_sector().to_vec()));

        // Wire drive byte 0 is ignored; LSN 630 -> drive 1, local LSN 0.
        let mut req = vec![opcode::READ, 0];
        req.extend(lsn_bytes(HDBDOS_SECTORS_PER_DISK as u32));
        let reply = feed_and_drain(&mut server, &req);
        assert_eq!(reply[0], error::OK);
        assert_eq!(&reply[1..1 + SECTOR_SIZE], &pattern_sector());

        // Same remap applies to WRITE.
        let sector = [0x42u8; SECTOR_SIZE];
        let checksum = checksum_of(&sector);
        let mut write_req = vec![opcode::WRITE, 0];
        write_req.extend(lsn_bytes(HDBDOS_SECTORS_PER_DISK as u32));
        write_req.extend(sector);
        write_req.push((checksum >> 8) as u8);
        write_req.push((checksum & 0xFF) as u8);
        let reply = feed_and_drain(&mut server, &write_req);
        assert_eq!(reply, vec![error::OK]);
        assert_eq!(server.image(1).unwrap().as_memory().unwrap(), &sector);
        assert_eq!(
            server.image(0).unwrap().as_memory().unwrap(),
            &[0xAAu8; SECTOR_SIZE]
        );
    }
}
