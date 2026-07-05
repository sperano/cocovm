//! FD-502 floppy disk controller cartridge: [`JvcDisk`] (JVC/.dsk image parsing)
//! and [`DiskCart`], the [`Cartridge`] that wires a [`crate::wd1773::WD1773`] and
//! four drive slots to the CoCo's DSKREG latch and the SCS/CTS windows.
//!
//! Hardware facts (DSKREG bit layout, drive/side resolution, the HALT*/NMI
//! control-line recomputation, JVC geometry) are cited from MAME source
//! (`coco_fdc.cpp`, `wd_fdc.cpp`/`.h`, `jvc_dsk.cpp`) in the doc comments below,
//! per the verified spec this module was built from.

use crate::cart::{Cartridge, IO_OPEN_BUS, RomPak, RomPakError};
use crate::wd1773::WD1773;

/// Number of physical drive slots the FD-502 exposes (DSKREG selects among
/// these four).
pub const DRIVE_COUNT: usize = 4;

// ---- JVC disk image (jvc_dsk.cpp) ------------------------------------------

/// Default sectors/track when the header omits byte 0 (MAME `jvc_dsk.cpp`).
pub const DEFAULT_SECTORS_PER_TRACK: usize = 18;
/// Default side count when the header omits byte 1.
pub const DEFAULT_SIDES: usize = 1;
/// Default sector-size code when the header omits byte 2: `128 << 1` = 256 bytes.
pub const DEFAULT_SECTOR_SIZE_CODE: u8 = 1;
/// Default first sector ID when the header omits byte 3.
pub const DEFAULT_FIRST_SECTOR_ID: u8 = 1;
/// Cap on tracks a Write Track (format) can grow an image to (MAME
/// `jvc_dsk.cpp`'s own track cap; spec-provided).
pub const MAX_FORMAT_TRACKS: usize = 82;

/// JVC header length is the file length modulo this — headerless images (the
/// common case) are an exact multiple of 256 bytes.
const HEADER_MODULUS: usize = 256;

/// OS-9 "LSN0" identification-sector fields, sniffed from headerless images to
/// disambiguate geometry a bare JVC-default parse gets wrong (MAME
/// `os9_dsk.cpp` `os9_format::find_size`). Explicit JVC headers keep full
/// authority — this sniff only ever runs when the header is absent.
mod os9_lsn0 {
    /// Sector length this module reads LSN0 from — a headerless image's
    /// sectors are the JVC default 256 bytes (`os9_dsk.cpp:90-95`).
    pub const LEN: usize = 256;
    /// DD.TOT (total sector count): 24-bit big-endian at offset 0x00
    /// (`os9_dsk.cpp:93`, `get_u24be(&os9_header[0x00])`).
    pub const TOT_OFFSET: usize = 0x00;
    /// DD.FMT (format byte) offset; only bit 0 is consulted (`os9_dsk.cpp:94`,
    /// `util::BIT(os9_header[0x10], 0) ? 2 : 1`).
    pub const FMT_OFFSET: usize = 0x10;
    /// DD.FMT bit 0: clear = 1 side, set = 2 sides.
    pub const FMT_SIDES_BIT: u8 = 0x01;
    /// DD.SPT (sectors per track): 16-bit big-endian at offset 0x11
    /// (`os9_dsk.cpp:95`, `get_u16be(&os9_header[0x11])`).
    pub const SPT_OFFSET: usize = 0x11;
}

/// Sniff a headerless image's first 256 bytes as an OS-9 LSN0 identification
/// sector and return the side count it declares (1 or 2), but only when the
/// fields are fully self-consistent with `file_len` and this crate's own JVC
/// default geometry — otherwise `None`, leaving the naive JVC-default parse
/// untouched.
///
/// Trusted only if: DD.SPT equals the JVC default (18 — this crate doesn't
/// support other sniffed geometries), `DD.TOT * 256 == file_len`, DD.TOT
/// divides evenly by `DD.SPT * sides`, and the implied track count is nonzero
/// and within [`MAX_FORMAT_TRACKS`] (MAME's largest floppy table entry is 80
/// tracks). This rejects both non-OS-9 images (an all-zero LSN0 fails the SPT
/// check) and disk-shaped-but-not-floppy images like a 1024-track cocosdc
/// dump (fails the track-count cap).
fn sniff_os9_sides(bytes: &[u8], file_len: usize) -> Option<usize> {
    let lsn0 = bytes.get(..os9_lsn0::LEN)?;
    let dd_tot = u32::from(lsn0[os9_lsn0::TOT_OFFSET]) << 16
        | u32::from(lsn0[os9_lsn0::TOT_OFFSET + 1]) << 8
        | u32::from(lsn0[os9_lsn0::TOT_OFFSET + 2]);
    let sides = if lsn0[os9_lsn0::FMT_OFFSET] & os9_lsn0::FMT_SIDES_BIT != 0 {
        2
    } else {
        1
    };
    let dd_spt =
        (u16::from(lsn0[os9_lsn0::SPT_OFFSET]) << 8 | u16::from(lsn0[os9_lsn0::SPT_OFFSET + 1])) as usize;

    if dd_spt != DEFAULT_SECTORS_PER_TRACK {
        return None;
    }
    let headerless_sector_size = 128usize << DEFAULT_SECTOR_SIZE_CODE;
    if dd_tot as usize * headerless_sector_size != file_len {
        return None;
    }
    let sectors_per_side_group = dd_spt * sides;
    if !(dd_tot as usize).is_multiple_of(sectors_per_side_group) {
        return None;
    }
    let implied_tracks = dd_tot as usize / sectors_per_side_group;
    if implied_tracks == 0 || implied_tracks > MAX_FORMAT_TRACKS {
        return None;
    }
    Some(sides)
}

/// Error constructing a [`JvcDisk`] from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JvcError {
    /// Header byte 4 (sector attribute flag) was nonzero: every sector would
    /// carry an extra prepended attribute byte, a JVC variant this
    /// implementation doesn't support.
    AttributeBytesUnsupported,
    /// The data portion (file length minus header) doesn't divide evenly into
    /// whole tracks of `sectors_per_track * sector_size * sides` bytes, or
    /// yields zero tracks.
    InvalidGeometry {
        file_len: usize,
        header_len: usize,
        sectors_per_track: usize,
        sides: usize,
        sector_size: usize,
    },
}

impl std::fmt::Display for JvcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JvcError::AttributeBytesUnsupported => {
                write!(f, "JVC images with per-sector attribute bytes are not supported")
            }
            JvcError::InvalidGeometry { file_len, header_len, sectors_per_track, sides, sector_size } => {
                write!(
                    f,
                    "JVC image geometry doesn't divide evenly into tracks: file_len={file_len}, \
                     header_len={header_len}, sectors_per_track={sectors_per_track}, sides={sides}, \
                     sector_size={sector_size}"
                )
            }
        }
    }
}

impl std::error::Error for JvcError {}

/// A JVC (`.dsk`/`.jvc`) floppy image, kept fully in memory.
///
/// Geometry comes from the optional trailing-length header (MAME
/// `jvc_dsk.cpp`): header length is `file_len % 256` (usually 0 — a headerless
/// image uses every default). Two-sided images interleave
/// track0-side0, track0-side1, track1-side0, … — see [`JvcDisk::sector_offset`].
#[derive(Debug, Clone)]
pub struct JvcDisk {
    sectors_per_track: usize,
    sides: usize,
    sector_size: usize,
    first_sector_id: u8,
    track_count: usize,
    header_len: usize,
    data: Vec<u8>,
    write_protected: bool,
    dirty: bool,
}

impl JvcDisk {
    /// Parse a raw JVC image. Rejects attribute-byte images and geometries that
    /// don't divide evenly into whole tracks.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, JvcError> {
        let file_len = bytes.len();
        let header_len = file_len % HEADER_MODULUS;
        let header = &bytes[..header_len];

        let sectors_per_track = header
            .first()
            .copied()
            .map(usize::from)
            .unwrap_or(DEFAULT_SECTORS_PER_TRACK);
        let mut sides = header.get(1).copied().map(usize::from).unwrap_or(DEFAULT_SIDES);
        let size_code = header.get(2).copied().unwrap_or(DEFAULT_SECTOR_SIZE_CODE);
        let sector_size = 128usize << size_code;
        let first_sector_id = header.get(3).copied().unwrap_or(DEFAULT_FIRST_SECTOR_ID);
        let attribute_flag = header.get(4).copied().unwrap_or(0);
        if attribute_flag != 0 {
            return Err(JvcError::AttributeBytesUnsupported);
        }

        let data_len = file_len - header_len;
        let track_bytes = sectors_per_track * sector_size * sides;
        if track_bytes == 0 || !data_len.is_multiple_of(track_bytes) {
            return Err(JvcError::InvalidGeometry {
                file_len,
                header_len,
                sectors_per_track,
                sides,
                sector_size,
            });
        }
        let mut track_count = data_len / track_bytes;

        // Headerless images only (an explicit JVC header keeps full authority):
        // sniff LSN0 for an OS-9 identification sector. A 2-sided disk whose
        // side-major bytes were parsed as 1 side needs its track count halved to
        // match (see `sniff_os9_sides`'s doc comment for the trust conditions).
        if header_len == 0
            && sides == DEFAULT_SIDES
            && sniff_os9_sides(&bytes, file_len) == Some(2)
        {
            sides = 2;
            track_count /= 2;
        }

        Ok(Self {
            sectors_per_track,
            sides,
            sector_size,
            first_sector_id,
            track_count,
            header_len,
            data: bytes,
            write_protected: false,
            dirty: false,
        })
    }

    pub fn track_count(&self) -> usize {
        self.track_count
    }

    pub fn sides(&self) -> usize {
        self.sides
    }

    pub fn sectors_per_track(&self) -> usize {
        self.sectors_per_track
    }

    pub fn sector_size(&self) -> usize {
        self.sector_size
    }

    pub fn first_sector_id(&self) -> u8 {
        self.first_sector_id
    }

    /// WD1773 Read Address size code: `size = 128 << code`.
    pub fn size_code(&self) -> u8 {
        ((self.sector_size / 128) as u32).trailing_zeros() as u8
    }

    pub fn write_protected(&self) -> bool {
        self.write_protected
    }

    pub fn set_write_protected(&mut self, write_protected: bool) {
        self.write_protected = write_protected;
    }

    /// Set since construction/last clear by a write to the image.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// The full image bytes (header included), e.g. for writing a modified
    /// disk back to its file.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Byte offset of `(track, side, sector_id)` in the in-memory image, or
    /// `None` if out of range.
    ///
    /// `offset = header + ((track * sides + side) * spt + (sector_id -
    /// first_id)) * sector_size` (spec-provided formula, matching MAME
    /// `jvc_dsk.cpp`'s sector lookup).
    pub fn sector_offset(&self, track: u8, side: u8, sector_id: u8) -> Option<usize> {
        let track = track as usize;
        let side = side as usize;
        if track >= self.track_count || side >= self.sides {
            return None;
        }
        let sector_index = sector_id.checked_sub(self.first_sector_id)? as usize;
        if sector_index >= self.sectors_per_track {
            return None;
        }
        let row = track * self.sides + side;
        Some(self.header_len + (row * self.sectors_per_track + sector_index) * self.sector_size)
    }

    pub fn read_bytes(&self, offset: usize, len: usize) -> &[u8] {
        &self.data[offset..offset + len]
    }

    pub fn write_byte(&mut self, offset: usize, val: u8) {
        self.data[offset] = val;
        self.dirty = true;
    }

    /// Lay down one formatted sector during a Write Track (DSKINI-style format).
    /// Writes `data` at `(track, side, sector_id)` if the geometry matches this
    /// image's own (side < sides(), sector_id in [first_sector_id,
    /// first_sector_id+sectors_per_track), 128<<size_code == sector_size()),
    /// growing the image with zero-filled tracks (capped at
    /// [`MAX_FORMAT_TRACKS`]) if `track` is beyond the current track count.
    /// Silently does nothing if the geometry doesn't match (foreign sector ID,
    /// wrong size code, side >= sides(), including a side-1 write on a
    /// single-sided image) or the cap is exceeded — real hardware has no error
    /// path for this, and JvcDisk can't represent a sector outside its own
    /// geometry (spec).
    pub fn format_sector(&mut self, track: u8, side: u8, sector_id: u8, size_code: u8, data: &[u8]) {
        let size = 128usize << size_code;
        if size != self.sector_size || side as usize >= self.sides {
            return;
        }
        let Some(sector_index) = sector_id.checked_sub(self.first_sector_id) else { return };
        if sector_index as usize >= self.sectors_per_track {
            return;
        }
        if track as usize >= self.track_count && !self.grow_to_track(track as usize) {
            return;
        }
        let offset = self
            .sector_offset(track, side, sector_id)
            .expect("geometry validated above; grow_to_track (if needed) covers `track`");
        self.data[offset..offset + size].copy_from_slice(data);
        self.dirty = true;
    }

    /// Extend the image with zero-filled tracks so `track` exists (lets Write
    /// Track format a blank/undersized image from nothing), capped at
    /// [`MAX_FORMAT_TRACKS`]. Returns `false` (image left unchanged) if `track`
    /// is beyond the cap; otherwise appends zero-filled track(s) after the
    /// current last track (row order is `track*sides+side`, so tracks are
    /// contiguous blocks — appending at the end is geometry-safe) and updates
    /// `track_count`.
    fn grow_to_track(&mut self, track: usize) -> bool {
        if track >= MAX_FORMAT_TRACKS {
            return false;
        }
        let track_bytes = self.sectors_per_track * self.sector_size * self.sides;
        let new_track_count = track + 1;
        self.data.resize(self.data.len() + (new_track_count - self.track_count) * track_bytes, 0);
        self.track_count = new_track_count;
        self.dirty = true;
        true
    }
}

// ---- DSKREG ($FF40) ---------------------------------------------------------

/// DSKREG latch bit assignments (`$FF40`; SCS writes at `$FF40-$FF47` all hit
/// it — MAME `coco_fdc.cpp` `dskreg_w`).
pub mod dskreg {
    /// Halt-enable: while set, the HALT* control line asserts whenever DRQ is
    /// low (see `DiskCart`'s `Cartridge::halt_asserted` implementation below).
    pub const HALT_ENABLE: u8 = 0x80;
    /// Drive-select 3 when no lower drive-select bit is set, else the side
    /// (head) select for drives 0-2.
    pub const DRIVE3_OR_SIDE: u8 = 0x40;
    /// Density select (1 = double). MAME wires this same bit to gate NMI on
    /// INTRQ; the `dden` pin the WD1773 actually sees is the inverse of this
    /// bit, which doesn't matter here since FM/MFM density isn't modelled —
    /// only the NMI-enable use of this bit is implemented.
    pub const DENSITY_AND_NMI_ENABLE: u8 = 0x20;
    /// Write precompensation select — stored, not acted on (spec).
    pub const WRITE_PRECOMP: u8 = 0x10;
    /// Motor on, all drives.
    pub const MOTOR_ON: u8 = 0x08;
    pub const DRIVE2: u8 = 0x04;
    pub const DRIVE1: u8 = 0x02;
    pub const DRIVE0: u8 = 0x01;
}

/// Resolve DSKREG's drive-select bits to a drive index (MAME `dskreg_w`):
/// bit2 wins if set, else bit1, else bit0, else bit6 (drive 3); `None` if none
/// of those four bits are set (no drive selected).
fn selected_drive(reg: u8) -> Option<usize> {
    if reg & dskreg::DRIVE2 != 0 {
        Some(2)
    } else if reg & dskreg::DRIVE1 != 0 {
        Some(1)
    } else if reg & dskreg::DRIVE0 != 0 {
        Some(0)
    } else if reg & dskreg::DRIVE3_OR_SIDE != 0 {
        Some(3)
    } else {
        None
    }
}

/// Resolve DSKREG's side (head) select: bit6 selects side 1, but only for
/// drives 0-2 — for drive 3, bit6 is the drive-select bit itself, not a side
/// select (spec).
fn selected_side(reg: u8, drive: Option<usize>) -> u8 {
    if reg & dskreg::DRIVE3_OR_SIDE != 0 && drive != Some(3) {
        1
    } else {
        0
    }
}

/// Free function (not a `DiskCart` method) so callers can borrow `drives`
/// mutably alongside a disjoint mutable borrow of `DiskCart::fdc` — going
/// through a `&mut self` method here would make the borrow checker see the
/// whole `DiskCart` as borrowed instead of just this one field.
fn selected_disk(
    drives: &mut [Option<JvcDisk>; DRIVE_COUNT],
    drive: Option<usize>,
) -> Option<&mut JvcDisk> {
    drive.and_then(|i| drives[i].as_mut())
}

// ---- WD1773 register offsets within the SCS window -------------------------

const STATUS_COMMAND_REG: u16 = 0xFF48;
const TRACK_REG: u16 = 0xFF49;
const SECTOR_REG: u16 = 0xFF4A;
const DATA_REG: u16 = 0xFF4B;
/// DSKREG mirrors across all of `$FF40-$FF47` (spec).
const DSKREG_BASE: u16 = 0xFF40;
const DSKREG_LAST: u16 = 0xFF47;

/// The FD-502: a WD1773 plus DSKREG plus four drive slots, serving the Disk
/// Extended Color BASIC ROM through the cartridge's CTS window.
pub struct DiskCart {
    rom: RomPak,
    fdc: WD1773,
    dskreg: u8,
    drives: [Option<JvcDisk>; DRIVE_COUNT],
    /// Previous state of the NMI line (`intrq && DENSITY_AND_NMI_ENABLE`), so
    /// [`DiskCart::update_lines`] can detect its rising edge.
    nmi_line: bool,
    /// A rising edge of the NMI line since the last [`Cartridge::take_nmi`].
    nmi_pending: bool,
}

impl std::fmt::Debug for DiskCart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiskCart")
            .field("dskreg", &self.dskreg)
            .field("fdc", &self.fdc)
            .field("mounted", &self.drives.each_ref().map(Option::is_some))
            .finish()
    }
}

impl DiskCart {
    /// Build a disk controller cartridge serving `rom` (Disk Extended Color
    /// BASIC, `disk11.rom`) through the CTS window, mirror-filled the same way
    /// a plain [`RomPak`] is (reused directly: same MAME `coco_pak_device`
    /// mirror-fill, same CTS-first/half-swap indexing). Never ties CART* to Q —
    /// like other Disk BASIC paks, BASIC finds it via the cold-start `DK` probe,
    /// not autostart.
    ///
    /// Reset state: DSKREG = 0, INTRQ clear, DRQ set (spec — a default-clear DRQ
    /// would spuriously assert HALT* the moment boot code sets DSKREG's
    /// halt-enable bit, before any command has run).
    pub fn new(rom: Box<[u8]>) -> Self {
        const AUTOSTART: bool = false;
        let rom = RomPak::from_bytes(&rom, AUTOSTART)
            .unwrap_or_else(|e: RomPakError| panic!("invalid disk controller ROM image: {e}"));
        Self {
            rom,
            fdc: WD1773::new(),
            dskreg: 0,
            drives: [None, None, None, None],
            nmi_line: false,
            nmi_pending: false,
        }
    }

    pub fn insert_disk(&mut self, drive: usize, disk: JvcDisk) {
        self.drives[drive] = Some(disk);
    }

    pub fn eject_disk(&mut self, drive: usize) {
        self.drives[drive] = None;
    }

    pub fn is_mounted(&self, drive: usize) -> bool {
        self.drives[drive].is_some()
    }

    /// The floppy in `drive`, if any (status display, write-back on eject).
    pub fn disk(&self, drive: usize) -> Option<&JvcDisk> {
        self.drives[drive].as_ref()
    }

    fn drive_index(&self) -> Option<usize> {
        selected_drive(self.dskreg)
    }

    fn side(&self) -> u8 {
        selected_side(self.dskreg, self.drive_index())
    }

    fn motor_on(&self) -> bool {
        self.dskreg & dskreg::MOTOR_ON != 0
    }


    /// Recompute the control lines the DSKREG/WD1773 pair drives, per MAME
    /// `coco_fdc.cpp update_lines`: called after every event that could change
    /// INTRQ, DRQ, or DSKREG (register access or a `tick`).
    ///
    /// 1. A high INTRQ clears DSKREG's halt-enable bit (hardware does this).
    /// 2. The NMI line is `intrq && DENSITY_AND_NMI_ENABLE`; an edge on *that*
    ///    line (not on INTRQ itself) is what queues an NMI.
    /// 3. HALT* is `!drq && HALT_ENABLE` — read live by [`Cartridge::halt_asserted`],
    ///    not cached here.
    fn update_lines(&mut self) {
        if self.fdc.intrq {
            self.dskreg &= !dskreg::HALT_ENABLE;
        }
        let nmi_line = self.fdc.intrq && self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0;
        if nmi_line && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = nmi_line;
    }
}

impl Cartridge for DiskCart {
    fn read(&mut self, addr: u16) -> u8 {
        let val = match addr {
            DSKREG_BASE..=DSKREG_LAST => IO_OPEN_BUS, // spec: reads here are open bus
            STATUS_COMMAND_REG => {
                let disk_present = self.drive_index().is_some_and(|i| self.drives[i].is_some());
                let motor_on = self.motor_on();
                self.fdc.read_status(disk_present, motor_on)
            }
            TRACK_REG => self.fdc.track,
            SECTOR_REG => self.fdc.sector,
            DATA_REG => self.fdc.read_data(),
            _ => IO_OPEN_BUS,
        };
        self.update_lines();
        val
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            DSKREG_BASE..=DSKREG_LAST => self.dskreg = val,
            STATUS_COMMAND_REG => {
                self.fdc.set_double_density(self.dskreg & dskreg::DENSITY_AND_NMI_ENABLE != 0);
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_command(val, disk, side);
            }
            TRACK_REG => self.fdc.track = val,
            SECTOR_REG => self.fdc.sector = val,
            DATA_REG => {
                let side = self.side();
                let idx = self.drive_index();
                let disk = selected_disk(&mut self.drives, idx);
                self.fdc.write_data(val, disk, side);
            }
            _ => {}
        }
        self.update_lines();
    }

    fn rom_read(&mut self, addr: u16) -> u8 {
        self.rom.rom_read(addr)
    }

    fn cart_line_ties_q(&self) -> bool {
        false
    }

    fn tick(&mut self, cycles: u32) {
        let side = self.side();
        let idx = self.drive_index();
        let disk = selected_disk(&mut self.drives, idx);
        self.fdc.tick(cycles, disk, side);
        self.update_lines();
    }

    fn halt_asserted(&self) -> bool {
        !self.fdc.drq && self.dskreg & dskreg::HALT_ENABLE != 0
    }

    fn take_nmi(&mut self) -> bool {
        std::mem::replace(&mut self.nmi_pending, false)
    }

    fn as_disk_cart(&mut self) -> Option<&mut DiskCart> {
        Some(self)
    }
}
