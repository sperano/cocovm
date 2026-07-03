//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! NOTE: a trait object (`Box<dyn Cartridge>`) is not `Serialize`, so `SystemBus`
//! and `Machine` don't yet derive serde. For save-states (`DESIGN.md` §9) this
//! becomes an enum or uses typetag — deferred.

/// Value read from the external ROM window when nothing drives the bus:
/// $00, matching real hardware / MAME coco3 (verified by MAME trace-diff,
/// 2026-07-02 — an empty slot's `LDD $C000` yields $0000, not $FFFF).
pub const ROM_OPEN_BUS: u8 = 0x00;

/// Value read from the cartridge I/O window ($FF40–$FF5F, SCS*) when nothing
/// drives the bus: floats high, like an unstrobed PIA input pin.
pub const IO_OPEN_BUS: u8 = 0xFF;

/// A device on the cartridge port.
pub trait Cartridge {
    /// Read cartridge I/O ($FF40–$FF5F, the SCS* decode).
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// True while this cartridge ties the expansion-port CART* line to the Q
    /// clock (~895 kHz): auto-start game paks do this so edges arrive
    /// continuously, which drives both the legacy PIA1 CB1 FIRQ path and the
    /// GIME EI0 input the same physical pin feeds. `SystemBus::hsync` polls
    /// this once per scanline — plenty to model a ~895 kHz signal, since the
    /// PIA/GIME only care that *an* edge keeps arriving. Disk-BASIC-style
    /// paks (no autostart) leave this false; they rely on BASIC's cold-start
    /// probe of `$C000`/`$C001` for `'D'`,`'K'` instead.
    fn cart_line_ties_q(&self) -> bool {
        false
    }
    /// Advance the cartridge's internal clocks by `cycles` CPU cycles. Called
    /// by the machine loop after every instruction (and once per burned cycle
    /// while the CPU is halted, so a device can pace work — the FDC's DRQ
    /// cadence — while it holds the HALT line).
    fn tick(&mut self, _cycles: u32) {}
    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// transfer handshake). Sampled at instruction boundaries.
    fn halt_asserted(&self) -> bool {
        false
    }
    /// Consume a pending NMI edge from the cartridge (the FD-502 gates the
    /// FDC's INTRQ onto the CPU NMI). Returns true at most once per edge.
    fn take_nmi(&mut self) -> bool {
        false
    }
}

/// No cartridge inserted.
#[derive(Debug, Clone, Default)]
pub struct EmptySlot;

impl Cartridge for EmptySlot {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
}

/// Largest image a ROM pak can hold: the external ROM window is 32K
/// (`$8000–$FFFF` under INIT0 MC1:MC0 = `11`).
pub const ROM_PAK_MAX_LEN: usize = 32 * 1024;

/// Base of the logical addresses `rom_read` receives (`$8000-$FDFF`).
const ROM_PAK_BASE: u16 = 0x8000;

/// Pak images are dumped CTS-window-first: file offset 0 is the byte at
/// `$C000`, and (for 32K carts) offset `$4000` is the byte at `$8000` once
/// INIT0 MC1:MC0 = `11` maps the second half in. The GIME routes cart banks
/// as `((bank & 3) ^ 2) * 0x2000` (MAME `gime.cpp` `update_memory`), i.e. the
/// two 16K halves are swapped relative to a flat `addr - $8000` view, so we
/// XOR the half-select bit when indexing. Invisible for ≤16K paks (the
/// mirror-fill makes both halves identical); load-bearing for 32K carts like
/// Arkanoid, whose entry code lives at file offset 0 expecting to be fetched
/// at `$C000`.
const ROM_PAK_HALF_SWAP: u16 = 0x4000;

/// Error constructing a [`RomPak`] from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RomPakError {
    /// The image had zero bytes.
    Empty,
    /// The image exceeded [`ROM_PAK_MAX_LEN`].
    TooLarge {
        /// The image's actual length, in bytes.
        len: usize,
    },
}

impl std::fmt::Display for RomPakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RomPakError::Empty => write!(f, "ROM pak image is empty"),
            RomPakError::TooLarge { len } => write!(
                f,
                "ROM pak image is {len} bytes, larger than the {ROM_PAK_MAX_LEN}-byte external ROM window"
            ),
        }
    }
}

impl std::error::Error for RomPakError {}

/// A cartridge ROM pak: a headerless raw dump (`.rom`/`.ccc`/`.bin`, 2K–32K) of
/// the kind sold for CoCo game/utility cartridges.
///
/// Undersized images are mirror-filled to the full 32K exactly as MAME's
/// `coco_pak_device::call_load` does, so `rom_read` needs no bounds logic
/// regardless of the original image size (fact 5). Indexing swaps the 16K
/// halves — see [`ROM_PAK_HALF_SWAP`].
pub struct RomPak {
    image: Box<[u8]>,
    /// Whether this pak ties the CART* line to Q (see
    /// [`Cartridge::cart_line_ties_q`]).
    autostart: bool,
}

impl std::fmt::Debug for RomPak {
    /// Elides the 32K image body; only its length and the autostart flag are
    /// interesting for debugging.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomPak")
            .field("image_len", &self.image.len())
            .field("autostart", &self.autostart)
            .finish()
    }
}

impl RomPak {
    /// Build a ROM pak from a raw image. Rejects empty images and images
    /// larger than [`ROM_PAK_MAX_LEN`]; anything in between is mirror-filled
    /// to 32K.
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, RomPakError> {
        if bytes.is_empty() {
            return Err(RomPakError::Empty);
        }
        if bytes.len() > ROM_PAK_MAX_LEN {
            return Err(RomPakError::TooLarge { len: bytes.len() });
        }

        let mut image = vec![0u8; ROM_PAK_MAX_LEN].into_boxed_slice();
        image[..bytes.len()].copy_from_slice(bytes);

        // Mirror-fill the rest with MAME `coco_pak_device`'s doubling loop:
        //   while read_length < cart_length {
        //       len = min(read_length, cart_length - read_length);
        //       copy buffer[0..len] to buffer[read_length..];
        //       read_length += len;
        //   }
        // Each copy lands at a multiple of the image length and copies a
        // prefix of an already-periodic buffer, so the result is byte-identical
        // to plain repetition (`image[i % len]`) for every image size — the
        // doubling is only an efficiency trick, kept in MAME's shape so the
        // provenance is obvious.
        let mut read_length = bytes.len();
        while read_length < ROM_PAK_MAX_LEN {
            let len = read_length.min(ROM_PAK_MAX_LEN - read_length);
            let (src, dst) = image.split_at_mut(read_length);
            dst[..len].copy_from_slice(&src[..len]);
            read_length += len;
        }

        Ok(Self { image, autostart })
    }
}

impl Cartridge for RomPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
}
