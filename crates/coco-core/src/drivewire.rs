//! DriveWire 4 — an in-process implementation of the *server* side of the
//! DriveWire protocol: a byte-stream RPC that lets NitrOS-9 (or DECB through
//! HDB-DOS) address disk images living on the host instead of real
//! hardware, one 256-byte sector at a time. This module contains only the
//! protocol engine: framing, opcodes, checksums, and the [`DWImage`] backing
//! store. The Becker-port register wiring ($FF41/$FF42) that feeds bytes into
//! [`DWServer::data_write`] and reads them from [`DWServer::data_read`] on the
//! CPU bus is a separate, later task.
//!
//! Opcode set, packet layout, checksum algorithm, and error codes are cited
//! from the DriveWire 4 Java server (`DWProtocolHandler.java`) and
//! pyDriveWire, cross-checked against NitrOS-9's driver-side implementation
//! (`dwio.asm`, `rbdw.asm`, `dwcheck.asm`), per the verified spec this
//! module was built from.
//!
//! The byte-level protocol state machine ([`data_write`](DWServer::data_write)
//! down to opcode dispatch) lives in the [`protocol`] submodule; the
//! READ/WRITE family's actual sector I/O lives in [`transfer`]; virtual
//! serial channels live in [`channel`] (state and queues) and [`serial`]
//! (wire requests and the host-side `channel_*` API).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

mod channel;
pub mod host;
mod image;
mod lifecycle;
mod protocol;
mod serial;
pub use channel::{
    CHANNEL_BUFFER_BYTES, CHANNEL_COUNT, ChannelDiagnostics, ChannelError, ChannelHandle,
    ChannelInfo,
};
pub use image::DWImage;
pub mod share;
mod transfer;

use channel::Channels;

use host::HostCompletion;
use host::HostExecutor;
use lifecycle::PendingHost;
use protocol::State;
use share::ShareSession;

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

    /// Virtual-serial family (`OP_SER*`), used by NitrOS-9's `scdwv`/`dwio`
    /// drivers for `/TERM`, `/N1`–`/N13`, and `/MIDI`. Wire values are from
    /// NitrOS-9's `defs/drivewire.d` and the Java server's `DWDefs.java`;
    /// request and reply shapes from the DriveWire specification's
    /// "Virtual Serial Channels" section. `drivewire::serial` implements
    /// them over the channels in `drivewire::channel`.
    ///
    /// Poll for channel input or a channel close: no payload, 2-byte reply
    /// (idle, a block of waiting bytes, or a close status).
    pub const SERREAD: u8 = 0x43;
    /// Read waiting channel bytes: 2-byte payload (channel, count). Reply is
    /// exactly `count` bytes.
    pub const SERREADM: u8 = 0x63;
    /// Write one byte to a channel: 2-byte payload (channel, data byte), no
    /// reply.
    pub const SERWRITE: u8 = 0xC3;
    /// Write a block to a channel: 2-byte header (channel, count), then
    /// `count` data bytes, no reply. Not sent by the pinned `scdwv`.
    pub const SERWRITEM: u8 = 0x64;
    /// Channel getstat notification: 2-byte payload (channel, statcode), no
    /// reply.
    pub const SERGETSTAT: u8 = 0x44;
    /// Channel setstat: 2-byte payload (channel, statcode), no reply. An
    /// [`super::SS_COMST`] statcode is followed by
    /// [`super::COMST_PAYLOAD_LEN`] more bytes; [`super::SS_OPEN`] and
    /// [`super::SS_CLOSE`] open and close the channel.
    pub const SERSETSTAT: u8 = 0xC4;
    /// Channel descriptor attached: 1-byte payload (channel), no reply.
    pub const SERINIT: u8 = 0x45;
    /// Channel descriptor detached: 1-byte payload (channel), no reply.
    pub const SERTERM: u8 = 0xC5;
    /// Fast write, one opcode per channel (channel = opcode minus
    /// [`FASTWRITE_BASE`]): 1-byte payload (data byte), no reply. The Java
    /// server accepts its 16 N and 16 window channels here (`$80`–`$9F`);
    /// bytes for channels beyond [`super::CHANNEL_COUNT`] are consumed and
    /// counted as unknown-channel operations.
    pub const FASTWRITE_BASE: u8 = 0x80;
    pub const FASTWRITE_LAST: u8 = 0x9F;
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

/// [`opcode::SERSETSTAT`]'s `SS.Open` statcode: SCF opened a path on the
/// channel (`scf.asm` `InvokeDriverOpen`; Java `DoOP_SERSETSTAT` `0x29`).
const SS_OPEN: u8 = 0x29;

/// [`opcode::SERSETSTAT`]'s `SS.Close` statcode: SCF closed a path on the
/// channel (`scf.asm` `CloseLastPath`; Java `DoOP_SERSETSTAT` `0x2A`).
const SS_CLOSE: u8 = 0x2A;

/// Byte length of a WRITE/REWRITE request body after the opcode: header +
/// 256 sector data bytes + 2-byte checksum.
const WRITE_BODY_LEN: usize = HEADER_LEN + SECTOR_SIZE + 2;

/// HDB-DOS flat addressing: sectors per virtual disk (35 tracks × 18
/// sectors/track — a standard DECB `.dsk` geometry). In HDB-DOS mode
/// ([`DWServer::set_hdbdos_mode`]) the wire drive byte is ignored;
/// `drive = lsn / HDBDOS_SECTORS_PER_DISK`, local
/// `lsn = lsn % HDBDOS_SECTORS_PER_DISK`.
pub const HDBDOS_SECTORS_PER_DISK: u64 = 630;

/// CoCo 3 maximum CPU clock (double-speed GIME POKE), in Hz. Duplicated
/// from the private `CPU_HZ`/speed-doubling logic in `lib.rs` (this module
/// must stay bus/host-free, so it can't import that) purely to document the
/// derivation of [`TRANSACTION_TIMEOUT_CYCLES`] that follows.
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
/// client to read" ([`DWServer::status_read`]). The Becker-port register
/// wiring itself ($FF41/$FF42) is out of scope for this module — this
/// constant only names the bit value this in-process server reports.
const STATUS_DATA_AVAILABLE: u8 = 0x02;

/// A wall-clock timestamp for [`opcode::TIME`] replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DWTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// A source of wall-clock time for [`opcode::TIME`], injected by the
/// frontend — `coco-core` has no host clock access of its own.
pub type DWClock = Box<dyn FnMut() -> DWTime + Send>;

/// [`DWServer`]'s default clock (before [`DWServer::set_clock`] is called):
/// an arbitrary but deterministic fixed date, since `coco-core` must stay
/// host-free. The frontend crate injects the real wall clock.
const DEFAULT_CLOCK_YEAR: u16 = 1990;
const DEFAULT_CLOCK_MONTH: u8 = 1;
const DEFAULT_CLOCK_DAY: u8 = 1;
const DEFAULT_CLOCK_HOUR: u8 = 0;
const DEFAULT_CLOCK_MINUTE: u8 = 0;
const DEFAULT_CLOCK_SECOND: u8 = 0;

fn default_clock() -> DWTime {
    DWTime {
        year: DEFAULT_CLOCK_YEAR,
        month: DEFAULT_CLOCK_MONTH,
        day: DEFAULT_CLOCK_DAY,
        hour: DEFAULT_CLOCK_HOUR,
        minute: DEFAULT_CLOCK_MINUTE,
        second: DEFAULT_CLOCK_SECOND,
    }
}

/// `#[serde(default = "...")]` for [`DWServer::clock`]; a restored server
/// falls back to this until the frontend calls [`DWServer::set_clock`].
fn default_dw_clock() -> DWClock {
    Box::new(default_clock)
}

/// Plain 16-bit sum of a 256-byte sector's bytes. Despite [`error::CRC`]'s
/// name, this is not an actual CRC. The maximum sum is 65_280, so a `u16`
/// wraparound is not possible.
fn checksum_of(sector: &[u8]) -> u16 {
    sector.iter().map(|&b| u16::from(b)).sum()
}

/// The DriveWire server: mounted images, the protocol state machine, and
/// the reply FIFO the Becker-port bus wiring drains from.
#[derive(Serialize, Deserialize)]
pub struct DWServer {
    /// Skipped: each mounted image can hold an open host `File` handle —
    /// remounted by path on restore through [`DWServer::reattach`].
    #[serde(skip)]
    drives: [Option<DWImage>; DRIVE_COUNT],
    #[serde(skip)]
    host: HostExecutor,
    #[serde(skip)]
    pending_host: Option<PendingHost>,
    #[serde(skip)]
    service_completions: VecDeque<HostCompletion>,
    /// Skipped: open host handles and leases cannot be restored. The
    /// frontend reinstalls the share table through [`DWServer::set_shares`].
    #[serde(skip)]
    shares: ShareSession,
    /// Set on a successful [`opcode::WRITE`]/[`opcode::REWRITE`]; cleared by
    /// [`DWServer::mount`]/[`DWServer::eject`].
    dirty: [bool; DRIVE_COUNT],
    reply: VecDeque<u8>,
    state: State,
    hdbdos: bool,
    /// Skipped: a closure has no serializable shape. Restored to
    /// [`default_dw_clock`] until the frontend calls [`DWServer::set_clock`]
    /// again.
    #[serde(skip, default = "default_dw_clock")]
    clock: DWClock,
    sectors_read: u64,
    sectors_written: u64,
    unknown_opcodes: u64,
    /// Count of virtual-serial-port opcodes handled (see [`opcode`]'s
    /// `OP_SER*`/[`opcode::FASTWRITE_BASE`] family), incremented once per
    /// top-level operation dispatched — not per byte consumed.
    vserial_ops: u64,
    /// Virtual serial channels. Restored channels hang up after the guest
    /// drains them (see `Channels::after_restore`); `#[serde(default)]`
    /// loads older save states with every channel closed.
    #[serde(default)]
    channels: Channels,
    /// Cycle stamp of the last byte fed through [`DWServer::data_write`], for
    /// the transaction timeout. `None` before the first byte ever arrives.
    last_byte_cycle: Option<u64>,
    /// Per-drive count of successful sector reads plus writes since
    /// construction, for the status bar's DriveWire activity light
    /// (`status_icons.rs`'s `ActivityLatch`) — mirrors [`Self::sectors_read`]/
    /// [`Self::sectors_written`] but per-drive and combined, matching what a
    /// single drive light should track. Bumped in `transfer::finish_read`/
    /// `transfer::finish_write` only on success; `NOT_READY`/`READ`/`WRITE`
    /// errors don't bump it. `#[serde(default)]` so an older save state
    /// without this field restores to all-zero counts rather than failing
    /// to load.
    #[serde(default)]
    drive_ops: [u64; DRIVE_COUNT],
}

impl DWServer {
    pub fn new() -> Self {
        Self {
            drives: std::array::from_fn(|_| None),
            host: HostExecutor::new(),
            pending_host: None,
            service_completions: VecDeque::new(),
            shares: ShareSession::default(),
            dirty: [false; DRIVE_COUNT],
            reply: VecDeque::new(),
            state: State::Idle,
            hdbdos: false,
            clock: Box::new(default_clock),
            sectors_read: 0,
            sectors_written: 0,
            unknown_opcodes: 0,
            vserial_ops: 0,
            channels: Channels::default(),
            last_byte_cycle: None,
            drive_ops: [0; DRIVE_COUNT],
        }
    }

    /// Mount `image` in `drive`, replacing anything already there and
    /// clearing its dirty flag.
    pub fn mount(&mut self, drive: usize, image: DWImage) {
        self.invalidate_drive_request(drive);
        self.drives[drive] = Some(image.into_async());
        self.dirty[drive] = false;
    }

    /// Unmount `drive`'s image, if any, and clear its dirty flag.
    pub fn eject(&mut self, drive: usize) {
        self.invalidate_drive_request(drive);
        self.drives[drive] = None;
        self.dirty[drive] = false;
    }

    /// Re-inject a mounted image after a snapshot restore without clearing `dirty[drive]`
    /// (unlike [`DWServer::mount`]) — the dirty flag is real state, not reset by remounting.
    pub fn reattach(&mut self, drive: usize, image: DWImage) {
        self.invalidate_drive_request(drive);
        self.drives[drive] = Some(image.into_async());
    }

    pub fn is_mounted(&self, drive: usize) -> bool {
        self.drives[drive].is_some()
    }

    /// The image mounted in `drive`, if any — mainly for tests (see
    /// [`DWImage::as_memory`]).
    pub fn image(&self, drive: usize) -> Option<&DWImage> {
        self.drives[drive].as_ref()
    }

    /// Enable/disable HDB-DOS flat addressing (see [`HDBDOS_SECTORS_PER_DISK`]);
    /// off by default, using the wire drive byte directly.
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

    /// Count of successful sector reads plus writes dispatched to `drive`
    /// so far (see `drive_ops`'s doc comment).
    pub fn drive_ops(&self, drive: usize) -> u64 {
        self.drive_ops[drive]
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
    pub fn set_clock(&mut self, clock: DWClock) {
        self.clock = clock;
    }

    /// Becker-port status register read: [`STATUS_DATA_AVAILABLE`] if a reply
    /// byte is queued, or `0` otherwise. Non-destructive, unlike
    /// [`DWServer::data_read`].
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
}

impl Default for DWServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "drivewire_test.rs"]
mod tests;
