//! JVC (`.dsk`/`.jvc`) floppy disk image parsing (MAME `jvc_dsk.cpp`).

use serde::{Deserialize, Serialize};

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
    /// [`JvcDisk::reattach_data`] only: the reattached file parses to a
    /// different geometry than the snapshot recorded — it changed shape
    /// (was reformatted, truncated, grown, …) since the snapshot was taken.
    GeometryChanged,
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
            JvcError::GeometryChanged => {
                write!(f, "reattached JVC image geometry doesn't match the snapshot's")
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JvcDisk {
    sectors_per_track: usize,
    sides: usize,
    sector_size: usize,
    first_sector_id: u8,
    track_count: usize,
    header_len: usize,
    /// Skipped: a mounted disk image's contents are media, referenced by
    /// path+hash in the snapshot container (a later phase) rather than
    /// embedded — floppy images can be copyrighted commercial software.
    /// Re-injected via [`JvcDisk::reattach_data`] (`docs/plan-save-states.md`).
    /// Deserializes to an empty `Vec` until reattached.
    #[serde(skip)]
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

    /// Restore-path-only: re-inject a mounted disk's raw bytes after a
    /// snapshot restore (`data` is `#[serde(skip)]` — mounted disk images
    /// are media, referenced by path+hash rather than embedded, since they
    /// can be copyrighted commercial software; `docs/plan-save-states.md`).
    /// Re-derives geometry from `bytes` exactly like [`JvcDisk::from_bytes`]
    /// and verifies it matches the geometry the snapshot recorded before
    /// setting `data` — [`JvcError::GeometryChanged`] means the file changed
    /// shape since the snapshot was taken.
    pub fn reattach_data(&mut self, bytes: Vec<u8>) -> Result<(), JvcError> {
        let reparsed = JvcDisk::from_bytes(bytes)?;
        if reparsed.sectors_per_track != self.sectors_per_track
            || reparsed.sides != self.sides
            || reparsed.sector_size != self.sector_size
            || reparsed.first_sector_id != self.first_sector_id
            || reparsed.track_count != self.track_count
            || reparsed.header_len != self.header_len
        {
            return Err(JvcError::GeometryChanged);
        }
        self.data = reparsed.data;
        Ok(())
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
