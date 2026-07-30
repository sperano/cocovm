//! Plain and bank-switched ROM paks: [`ROMPak`] (up to 32K, fixed) and
//! [`BankedROMPak`] (up to 128K, `$FF40`-latched 16K window).

use serde::{Deserialize, Serialize};

use super::{Cartridge, IO_OPEN_BUS};

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

/// Error constructing a [`ROMPak`] from a raw image.
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
#[derive(Serialize, Deserialize)]
pub struct ROMPak {
    /// Skipped: COPYRIGHTED pak bytes (always a 32K mirror-fill) never
    /// travel through a snapshot; re-injected on restore via
    /// [`ROMPak::reattach_image`] (`docs/plan-save-states.md`). Deserializes
    /// to an empty `Box<[u8]>` until reattached.
    #[serde(skip)]
    image: Box<[u8]>,
    /// Whether this pak ties the CART* line to Q (see
    /// [`Cartridge::cart_line_ties_q`]).
    autostart: bool,
}

impl std::fmt::Debug for ROMPak {
    /// Elides the 32K image body; only its length and the autostart flag are
    /// interesting for debugging.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomPak")
            .field("image_len", &self.image.len())
            .field("autostart", &self.autostart)
            .finish()
    }
}

impl ROMPak {
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

        Ok(Self {
            image: mirror_fill(bytes, ROM_PAK_MAX_LEN),
            autostart,
        })
    }

    /// Restore-path-only: re-inject this pak's image after a snapshot
    /// restore, leaving `autostart` untouched (unlike
    /// [`ROMPak::from_bytes`], which always takes a fresh value for it) — the
    /// deserialized `autostart` is itself the restored machine state
    /// (`docs/plan-save-states.md`). Same validation and mirror-fill as
    /// [`ROMPak::from_bytes`].
    pub fn reattach_image(&mut self, bytes: &[u8]) -> Result<(), RomPakError> {
        if bytes.is_empty() {
            return Err(RomPakError::Empty);
        }
        if bytes.len() > ROM_PAK_MAX_LEN {
            return Err(RomPakError::TooLarge { len: bytes.len() });
        }
        self.image = mirror_fill(bytes, ROM_PAK_MAX_LEN);
        Ok(())
    }
}

/// Copy `bytes` into a `total_len` buffer and mirror-fill the rest with MAME
/// `cococart_slot_device::call_load`'s doubling loop:
///   while read_length < cart_length {
///       len = min(read_length, cart_length - read_length);
///       copy buffer[0..len] to buffer[read_length..];
///       read_length += len;
///   }
/// Each copy lands at a multiple of the image length and copies a prefix of
/// an already-periodic buffer, so the result is byte-identical to plain
/// repetition (`image[i % len]`) for every image size — the doubling is only
/// an efficiency trick, kept in MAME's shape so the provenance is obvious.
/// The slot device runs this same loop for every pak type, so [`ROMPak`]
/// (32K) and [`BankedROMPak`] (128K) share it.
fn mirror_fill(bytes: &[u8], total_len: usize) -> Box<[u8]> {
    let mut image = vec![0u8; total_len].into_boxed_slice();
    image[..bytes.len()].copy_from_slice(bytes);
    let mut read_length = bytes.len();
    while read_length < total_len {
        let len = read_length.min(total_len - read_length);
        let (src, dst) = image.split_at_mut(read_length);
        dst[..len].copy_from_slice(&src[..len]);
        read_length += len;
    }
    image
}

impl Cartridge for ROMPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn rom_read(&mut self, addr: u16) -> u8 {
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn rom_peek(&self, addr: u16) -> u8 {
        // Pure array fetch — identical to `rom_read`, no side effects.
        self.image[((addr - ROM_PAK_BASE) ^ ROM_PAK_HALF_SWAP) as usize]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
}

/// The banked pak's CTS window: the `$FF40` latch slides a **16K** view over
/// the image (MAME `coco_pak.cpp` `coco_pak_banked_device::get_cart_size()`
/// = `0x4000`), unlike the plain [`ROMPak`]'s fixed 32K.
pub const BANKED_PAK_WINDOW_LEN: usize = 16 * 1024;

/// Largest banked-pak image: MAME's banked cart ROM region is 128K
/// (`coco_pak.cpp` `ROM_REGION(0x20000, ...)`), 8 banks of 16K. The bank
/// latch wraps modulo this, so undersized images (mirror-filled to 128K,
/// like every pak) repeat across the unused banks.
pub const BANKED_PAK_MAX_LEN: usize = 128 * 1024;

/// The bank latch address (MAME `coco_pak_banked_device::scs_write` case 0
/// of the SCS window).
const BANKED_PAK_BANK_REG: u16 = 0xFF40;

/// Error constructing a [`BankedROMPak`] from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankedPakError {
    /// The image had zero bytes.
    Empty,
    /// The image exceeded [`BANKED_PAK_MAX_LEN`].
    TooLarge {
        /// The image's actual length, in bytes.
        len: usize,
    },
}

impl std::fmt::Display for BankedPakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BankedPakError::Empty => write!(f, "banked ROM pak image is empty"),
            BankedPakError::TooLarge { len } => write!(
                f,
                "banked ROM pak image is {len} bytes, larger than the {BANKED_PAK_MAX_LEN}-byte banked address space"
            ),
        }
    }
}

impl std::error::Error for BankedPakError {}

/// A banked ROM pak: the RoboCop/Predator bank-switch circuit that the Games
/// Master Cartridge reuses (MAME `coco_pak.cpp` `coco_pak_banked_device`).
/// A whole-byte write to `$FF40` selects which 16K page of the (up to 128K)
/// image the external ROM window shows; the effective bank is
/// `(latch * 16K) mod 128K` (MAME `cts_read`'s
/// `(m_pos * 0x4000) % m_eprom->bytes()`), and the latch resets to bank 0 on
/// the RESET* line (`device_reset`).
#[derive(Serialize, Deserialize)]
pub struct BankedROMPak {
    /// Skipped: COPYRIGHTED pak bytes (always a 128K mirror-fill) never
    /// travel through a snapshot; re-injected on restore via
    /// [`BankedROMPak::reattach_image`] (`docs/plan-save-states.md`).
    /// Deserializes to an empty `Box<[u8]>` until reattached.
    #[serde(skip)]
    image: Box<[u8]>,
    /// The raw `$FF40` latch byte (`m_pos`) — masking happens at read time,
    /// via the modulo above.
    bank: u8,
    /// Whether this pak ties the CART* line to Q (see
    /// [`Cartridge::cart_line_ties_q`]).
    autostart: bool,
}

impl std::fmt::Debug for BankedROMPak {
    /// Elides the 128K image body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BankedRomPak")
            .field("bank", &self.bank)
            .field("autostart", &self.autostart)
            .finish()
    }
}

impl BankedROMPak {
    /// Build a banked ROM pak from a raw image. Rejects empty images and
    /// images larger than [`BANKED_PAK_MAX_LEN`]; anything in between is
    /// mirror-filled to the full 128K (MAME loads every pak type through the
    /// same doubling loop — see [`mirror_fill`]).
    pub fn from_bytes(bytes: &[u8], autostart: bool) -> Result<Self, BankedPakError> {
        if bytes.is_empty() {
            return Err(BankedPakError::Empty);
        }
        if bytes.len() > BANKED_PAK_MAX_LEN {
            return Err(BankedPakError::TooLarge { len: bytes.len() });
        }
        Ok(Self {
            image: mirror_fill(bytes, BANKED_PAK_MAX_LEN),
            bank: 0,
            autostart,
        })
    }

    /// Restore-path-only: re-inject this pak's image after a snapshot
    /// restore, leaving `bank`/`autostart` untouched — both are themselves
    /// restored machine state (`docs/plan-save-states.md`). Same validation
    /// and mirror-fill as [`BankedROMPak::from_bytes`].
    pub fn reattach_image(&mut self, bytes: &[u8]) -> Result<(), BankedPakError> {
        if bytes.is_empty() {
            return Err(BankedPakError::Empty);
        }
        if bytes.len() > BANKED_PAK_MAX_LEN {
            return Err(BankedPakError::TooLarge { len: bytes.len() });
        }
        self.image = mirror_fill(bytes, BANKED_PAK_MAX_LEN);
        Ok(())
    }
}

impl Cartridge for BankedROMPak {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, addr: u16, val: u8) {
        if addr == BANKED_PAK_BANK_REG {
            self.bank = val;
        }
    }
    fn rom_read(&mut self, addr: u16) -> u8 {
        // The 16K window mirrors identically into both halves of the 32K
        // external map, so the GIME half-swap (see [`ROM_PAK_HALF_SWAP`])
        // flips a bit the window mask discards — it drops out entirely.
        let window = usize::from(addr - ROM_PAK_BASE) & (BANKED_PAK_WINDOW_LEN - 1);
        let base = usize::from(self.bank) * BANKED_PAK_WINDOW_LEN % BANKED_PAK_MAX_LEN;
        self.image[base + window]
    }
    fn cart_line_ties_q(&self) -> bool {
        self.autostart
    }
    fn reset(&mut self) {
        self.bank = 0;
    }
}
