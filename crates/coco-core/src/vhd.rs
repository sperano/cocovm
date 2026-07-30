//! VHD — the MAME-compatible CoCo "virtual hard disk" device: a flat sector
//! image addressed through a tiny 7-byte register window ($FF80–$FF86), used
//! by NitrOS-9's `emudsk` driver in place of a real WD1773/floppy stack.
//! Register layout, command/status codes, and sector geometry are cited from
//! MAME `coco_vhd.cpp` (device registers and command dispatch) and
//! `coco.cpp`/`coco3.cpp` (I/O page placement) in the doc comments below, per
//! the verified spec this module was built from; the driver-side contract
//! (24-bit LRN, 256-byte sectors) is corroborated by NitrOS-9 `emudsk.asm`.
//!
//! This module holds only data and file I/O: [`VHD`]/`VHDDrive`/[`VHDImage`]
//! know nothing about the CPU bus. The actual command execution — which needs
//! to transfer sector data through the GIME-translated logical address space
//! — lives in `SystemBus`'s private methods in `bus.rs`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use serde::{Deserialize, Serialize};

/// Number of drives the device exposes; `$FF86` selects between them.
pub const DRIVE_COUNT: usize = 2;

/// Fixed sector size for VHD images (MAME `coco_vhd.cpp`): a flat file with
/// sector N at byte offset `SECTOR_SIZE * N`, no header, no metadata.
pub const SECTOR_SIZE: usize = 256;

/// Open-bus value returned by registers that don't answer while their drive
/// is deselected (`$FF80`–`$FF85`), and unconditionally by `$FF86` itself.
const OPEN_BUS: u8 = 0xFF;

/// Commands written to `$FF83` (MAME `coco_vhd.cpp` `vhd_w`).
pub mod command {
    /// Read the sector at the current LRN into the buffer address.
    pub const READ: u8 = 0x00;
    /// Write the buffer address's 256 bytes to the sector at the current LRN.
    pub const WRITE: u8 = 0x01;
    /// Flush the backing image to its storage medium.
    pub const FLUSH: u8 = 0x02;
}

/// Status codes read from `$FF83` (MAME `coco_vhd.cpp`).
pub mod status {
    /// Command completed successfully.
    pub const OK: u8 = 0x00;
    /// No VHD image is attached to the selected drive.
    pub const NO_VHD: u8 = 0x02;
    /// The command's file access failed (host I/O error).
    pub const IO_ERROR: u8 = 0x05;
    /// The byte written to `$FF83` isn't a recognized command.
    pub const UNKNOWN_COMMAND: u8 = 0xFE;
    /// Power-on/insert state: no command has run yet.
    pub const POWER_ON: u8 = 0xFF;
}

/// A VHD backing image: either an in-memory buffer (tests — small, cheap to
/// construct and assert against) or a real file, accessed by seeking rather
/// than loaded whole (real VHD images run from hundreds of MB to several GB).
pub enum VHDImage {
    Memory(Vec<u8>),
    File(File),
}

impl VHDImage {
    /// Current length of the backing image in bytes.
    fn len(&self) -> io::Result<u64> {
        match self {
            VHDImage::Memory(bytes) => Ok(bytes.len() as u64),
            VHDImage::File(file) => Ok(file.metadata()?.len()),
        }
    }

    /// Read up to `buf.len()` bytes starting at `offset`, returning the
    /// number of bytes actually available (`0` if `offset` is at or past the
    /// image's current length). Running off the end of the image partway
    /// through is a normal short read, not an error — only a genuine I/O
    /// error returns `Err`.
    pub(crate) fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.len()?;
        if offset >= len {
            return Ok(0);
        }
        let available = (len - offset) as usize;
        let want = buf.len().min(available);
        match self {
            VHDImage::Memory(bytes) => {
                let start = offset as usize;
                buf[..want].copy_from_slice(&bytes[start..start + want]);
            }
            VHDImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.read_exact(&mut buf[..want])?;
            }
        }
        Ok(want)
    }

    /// Zero-extend the image to at least `len` bytes if it is currently
    /// shorter than that; a no-op otherwise.
    pub(crate) fn extend_to(&mut self, len: u64) -> io::Result<()> {
        if self.len()? >= len {
            return Ok(());
        }
        match self {
            VHDImage::Memory(bytes) => bytes.resize(len as usize, 0),
            VHDImage::File(file) => file.set_len(len)?,
        }
        Ok(())
    }

    /// Write `buf` at `offset`, growing the image if needed to fit (for
    /// `Memory`, resizing zero-fills any newly created gap before `offset`;
    /// for `File`, seeking past the current end and writing extends it the
    /// same way a real file does).
    pub(crate) fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        match self {
            VHDImage::Memory(bytes) => {
                let end = offset as usize + buf.len();
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[offset as usize..end].copy_from_slice(buf);
                Ok(())
            }
            VHDImage::File(file) => {
                file.seek(SeekFrom::Start(offset))?;
                file.write_all(buf)
            }
        }
    }

    /// Flush the backing file to disk; a no-op for an in-memory test image.
    pub(crate) fn flush(&mut self) -> io::Result<()> {
        match self {
            VHDImage::Memory(_) => Ok(()),
            VHDImage::File(file) => file.flush(),
        }
    }

    /// The image's raw bytes, for inspection — only meaningful for the
    /// in-memory variant (tests construct one, mount it, then read this back
    /// to check what a command wrote); `None` for a file-backed image.
    pub fn as_memory(&self) -> Option<&[u8]> {
        match self {
            VHDImage::Memory(bytes) => Some(bytes),
            VHDImage::File(_) => None,
        }
    }
}

/// Per-drive register state: the 24-bit logical record number and 16-bit CPU
/// buffer address latched by writes to `$FF80–$FF82`/`$FF84–$FF85`, the last
/// command's outcome (`$FF83` read), and the mounted image, if any.
#[derive(Serialize, Deserialize)]
pub(crate) struct VHDDrive {
    /// 24-bit logical record (sector) number; the top 8 bits of the `u32` are
    /// always 0.
    pub(crate) lrn: u32,
    /// 16-bit CPU logical address the next transfer reads from/writes to.
    pub(crate) buffer_addr: u16,
    pub(crate) status: u8,
    /// Skipped: an open host `File` handle. Remounted by path on restore via
    /// [`VHD::reattach_image`] (`docs/plan-save-states.md`).
    #[serde(skip)]
    pub(crate) image: Option<VHDImage>,
}

impl VHDDrive {
    fn new() -> Self {
        Self { lrn: 0, buffer_addr: 0, status: status::NO_VHD, image: None }
    }
}

/// The VHD device: two independent drives plus the shared `$FF86`
/// drive-select latch.
#[derive(Serialize, Deserialize)]
pub struct VHD {
    pub(crate) drives: [VHDDrive; DRIVE_COUNT],
    /// Raw value last written to `$FF86`. `0`/`1` select a drive; anything
    /// else deselects both (see [`VHD::selected_drive`]).
    select: u8,
    /// Reentrancy guard for command execution (`SystemBus::vhd_execute_command`
    /// in `bus.rs`): our own addition, not modeled by MAME — needed because
    /// a command's byte-transfer loop can itself write to `$FF83` if the CPU
    /// buffer address happens to land in the VHD's own I/O page. Guards only
    /// command *execution*; register writes (LRN/buffer address) during a
    /// transfer are not gated and behave normally.
    pub(crate) busy: bool,
    /// Per-drive count of READ/WRITE/FLUSH commands dispatched since
    /// construction — our own addition, not modeled by MAME, for the
    /// status bar's VHD activity light (`status_icons.rs`'s `ActivityLatch`).
    /// Bumped in `SystemBus::vhd_execute_command` only for a real dispatch
    /// (past the busy/mounted guards); an unknown command or an unmounted
    /// drive leaves it untouched. `#[serde(default)]` so an older save
    /// state without this field restores to all-zero counts rather than
    /// failing to load.
    #[serde(default)]
    pub(crate) access_counts: [u64; DRIVE_COUNT],
}

impl VHD {
    /// Both drives start unmounted (status [`status::NO_VHD`]).
    ///
    /// `select` defaults to drive 0. This is an inferred default, not a
    /// verified hardware fact: no source available for this implementation
    /// states the drive-select latch's power-on value. Drive 0 selected
    /// matches a typical zeroed-register reset state and is the natural
    /// default a DOS would assume, but should be treated as low-confidence
    /// until confirmed against real hardware or MAME's device reset code.
    pub fn new() -> Self {
        Self {
            drives: [VHDDrive::new(), VHDDrive::new()],
            select: 0,
            busy: false,
            access_counts: [0; DRIVE_COUNT],
        }
    }

    /// The currently addressed drive, or `None` if `$FF86` last saw a value
    /// other than 0 or 1 (deselected).
    pub fn selected_drive(&self) -> Option<usize> {
        match self.select {
            0 => Some(0),
            1 => Some(1),
            _ => None,
        }
    }

    /// Mount `image` in `drive`: status becomes [`status::POWER_ON`], and the
    /// LRN/buffer-address registers reset to 0 (MAME `coco_vhd.cpp` image
    /// load).
    pub fn insert(&mut self, drive: usize, image: VHDImage) {
        let d = &mut self.drives[drive];
        d.image = Some(image);
        d.status = status::POWER_ON;
        d.lrn = 0;
        d.buffer_addr = 0;
    }

    /// Unmount `drive`'s image; status reverts to [`status::NO_VHD`].
    pub fn eject(&mut self, drive: usize) {
        let d = &mut self.drives[drive];
        d.image = None;
        d.status = status::NO_VHD;
    }

    /// Restore-path-only: re-inject a mounted image after a snapshot
    /// restore, WITHOUT resetting `lrn`/`buffer_addr`/`status` the way
    /// [`VHD::insert`] does — all three are themselves restored machine
    /// state, exactly as deserialized (`docs/plan-save-states.md`).
    pub fn reattach_image(&mut self, drive: usize, image: VHDImage) {
        self.drives[drive].image = Some(image);
    }

    pub fn is_mounted(&self, drive: usize) -> bool {
        self.drives[drive].image.is_some()
    }

    /// The image mounted in `drive`, if any — an inspection accessor mainly
    /// useful for tests (see [`VHDImage::as_memory`]).
    pub fn image(&self, drive: usize) -> Option<&VHDImage> {
        self.drives[drive].image.as_ref()
    }

    /// Count of READ/WRITE/FLUSH commands dispatched to `drive` so far (see
    /// `access_counts`'s doc comment). Out-of-range `drive` reads as `0`
    /// rather than panicking, so a caller iterating over its own drive count
    /// (e.g. the status bar's UI-side drive list) can't be made to panic by
    /// a mismatch against [`DRIVE_COUNT`].
    pub fn access_count(&self, drive: usize) -> u64 {
        self.access_counts.get(drive).copied().unwrap_or(0)
    }

    /// `$FF80–$FF82`/`$FF84–$FF85` read: MAME implements no readback for
    /// these registers. They read `0` while a drive is selected, open bus
    /// while deselected.
    pub fn read_lrn_or_buffer(&self) -> u8 {
        if self.selected_drive().is_some() { 0 } else { OPEN_BUS }
    }

    /// `$FF83` read: the selected drive's last command status, or open bus
    /// while deselected.
    pub fn read_status(&self) -> u8 {
        match self.selected_drive() {
            Some(drive) => self.drives[drive].status,
            None => OPEN_BUS,
        }
    }

    /// `$FF80` write (LRN high byte, bits 16–23). Dropped while deselected.
    pub fn write_lrn_hi(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.lrn = (d.lrn & 0x00FFFF) | (u32::from(val) << 16);
        }
    }

    /// `$FF81` write (LRN mid byte, bits 8–15). Dropped while deselected.
    pub fn write_lrn_mid(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.lrn = (d.lrn & 0xFF00FF) | (u32::from(val) << 8);
        }
    }

    /// `$FF82` write (LRN low byte, bits 0–7). Dropped while deselected.
    pub fn write_lrn_lo(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.lrn = (d.lrn & 0xFFFF00) | u32::from(val);
        }
    }

    /// `$FF84` write (buffer address high byte). Dropped while deselected.
    pub fn write_buffer_hi(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.buffer_addr = (d.buffer_addr & 0x00FF) | (u16::from(val) << 8);
        }
    }

    /// `$FF85` write (buffer address low byte). Dropped while deselected.
    pub fn write_buffer_lo(&mut self, val: u8) {
        if let Some(drive) = self.selected_drive() {
            let d = &mut self.drives[drive];
            d.buffer_addr = (d.buffer_addr & 0xFF00) | u16::from(val);
        }
    }

    /// `$FF86` write: `0`/`1` select drive 0/1; any other value deselects
    /// both drives until this is rewritten with 0 or 1.
    pub fn write_select(&mut self, val: u8) {
        self.select = val;
    }
}

impl Default for VHD {
    fn default() -> Self {
        Self::new()
    }
}
