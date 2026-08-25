//! `SAM` — the MC6883 Synchronous Address Multiplexer's primary memory map, for
//! the plain CoCo 1/2 machine (no GIME). See
//!
//! This is a standalone model, deliberately **not** shared with the GIME's own
//! SAM-compatibility overlay (`gime.rs::write_sam`/`SAM_BASE`..): the GIME keeps
//! modelling its own CoCo-3-compatible strobes independently, and this file
//! duplicates the handful of constants that describe (verified against MAME
//! `6883sam.cpp`, the CoCo 2 NTSC Service Manual SAM register map pp. 8-10, and
//! Bob Russell's memory map — see). The small duplication
//! is intentional: it keeps the CoCo 3 path completely untouched.

use serde::{Deserialize, Serialize};

/// Base/last of the SAM control-strobe address range ($FFC0–$FFDF). Every SAM
/// bit is a pair of addresses: the even one clears it, the odd one sets it —
/// the data written is ignored (service manual p. 8; MAME `6883sam.h`
/// `alter_sam_state`).
pub const STROBE_BASE: u16 = 0xFFC0;
pub const STROBE_LAST: u16 = 0xFFDF;

/// $FF00–$FF7E: PIA0, PIA1, cart SCS* ($FF40–$FF5F), and the cart SCS*
/// extension some cartridges decode ($FF60–$FF7E, e.g. the Sound/Speech
/// Cartridge's $FF7D/$FF7E — `docs/cartridges.md` "Carts can decode
/// addresses outside SCS") — decoded by the bus, not `SAM` itself (`SAM::map`
/// only reports that this range is I/O).
const IO_BASE: u16 = 0xFF00;
/// $FF7F–$FFBF: no GIME (hence no MPI-style `$FF7F` decode either) on these
/// machines — open bus.
const OPEN_BUS_BASE: u16 = 0xFF7F;
const OPEN_BUS_LAST: u16 = 0xFFBF;

/// $8000–$9FFF: Extended Color BASIC ROM window.
const EXT_ROM_BASE: u16 = 0x8000;
const EXT_ROM_LAST: u16 = 0x9FFF;
/// $A000–$BFFF: Color BASIC ROM window.
const BAS_ROM_BASE: u16 = 0xA000;
const BAS_ROM_LAST: u16 = 0xBFFF;
/// $C000–$FEFF: cartridge (CTS*) ROM window.
const CART_ROM_BASE: u16 = 0xC000;
const CART_ROM_LAST: u16 = 0xFEFF;

/// $FFE0–$FFFF: the top 32 bytes of the window, always mirroring
/// $BFE0–$BFFF (the top of the Color BASIC ROM, including the 6809 hardware
/// vectors) — independent of TY, M1, or P1. The service manual documents only
/// the narrower $FFF2–$FFFF -> $BFF2–$BFFF vector mirror, but MAME's decode
/// (`6883sam.cpp`: `offset >= 0xffe0` selects ROM slot 1 with `offset &
/// 0x1fff`) is the full 32 bytes; follow MAME.
const VECTOR_MIRROR_BASE: u16 = 0xFFE0;
/// `BAS_ROM` offset the vector mirror starts at: $BFE0 - $A000.
const BAS_MIRROR_OFFSET: usize = 0xBFE0 - BAS_ROM_BASE as usize;

/// Each F-bit (display page) step is 512 bytes: `display_base = f * PAGE_UNIT`.
pub const PAGE_UNIT: usize = 512;

/// Where a CPU address decodes to, per [`SAM::map`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SAMTarget {
    /// Physical RAM address (already includes any P1 banking).
    Ram(usize),
    /// Offset into the Extended Color BASIC ROM half of the flat 32K image
    /// (`0..0x2000`, image offset `off` directly).
    RomExt(usize),
    /// Offset into the Color BASIC ROM half of the flat 32K image
    /// (`0..0x2000`+; image offset `0x2000 + off`).
    RomBas(usize),
    /// Offset from $C000 into the cartridge CTS* ROM window.
    Cart(usize),
    /// PIA0 ($FF00–$FF1F), PIA1 ($FF20–$FF3F), cart SCS* ($FF40–$FF5F) plus
    /// its extension ($FF60–$FF7E), or a SAM control strobe ($FFC0–$FFDF) —
    /// the bus decodes further by address.
    Io,
    /// $FF7F–$FFBF: no GIME registers exist on these machines.
    OpenBus,
}

/// MC6883 SAM register state. All 16 bits
/// power up clear. Fields are `pub` (matching `gime::GIME`'s style) so tests
/// can inspect latched state directly, same as `sam_video.rs` does for the
/// GIME's compatibility overlay; [`SAM::map`]/[`SAM::display_base`]/
/// [`SAM::v_bits`]/[`SAM::cpu_fast`] are the API real callers use.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SAM {
    /// VDG-counter mode bits V0-V2, packed as `V2:V1:V0` (0-7). Latched but
    /// not consulted here — `video.rs`'s legacy-graphics vertical cadence
    /// lookup uses it.
    pub v: u8,
    /// Display-offset bits F0-F6 (0-127); `display_base = f * PAGE_UNIT`.
    pub f: u8,
    /// Page #1: banks the upper 32K of RAM into $0000-$7FFF. Only effective
    /// when TY=0 and 64K (`m1`).
    pub p1: bool,
    /// CPU rate, address-dependent half (ROM fast / RAM slow on real
    /// hardware). See [`SAM::cpu_fast`]'s KNOWN GAP note.
    pub r0: bool,
    /// CPU rate, unconditional-double-speed half.
    pub r1: bool,
    /// Memory-size bit 0 (4K/16K/32K-64K, with `m1`).
    pub m0: bool,
    /// Memory-size bit 1; used here only as the plan's 64K/"not 64K" proxy
    /// (see [`SAM::is_64k`]) for TY's all-RAM precondition and P1 banking.
    pub m1: bool,
    /// Map type: false = ROM map, true = all-RAM (system ROM disabled). Color
    /// BASIC Unravelled's appendix has $FFDE/$FFDF backwards; MAME, Bob
    /// Russell, and the service manual (p. 8) agree TY=1 (set, $FFDF) is
    /// all-RAM.
    pub ty: bool,
}

impl SAM {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a SAM control-strobe write ($FFC0–$FFDF): the even address in each
    /// pair clears the bit, the odd sets it; the data written is irrelevant.
    pub fn write_strobe(&mut self, addr: u16) {
        if !(STROBE_BASE..=STROBE_LAST).contains(&addr) {
            return;
        }
        let index = (addr - STROBE_BASE) / 2;
        let set = (addr - STROBE_BASE) & 1 != 0;
        match index {
            0..=2 => set_bit(&mut self.v, index as u8, set),
            3..=9 => set_bit(&mut self.f, (index - 3) as u8, set),
            10 => self.p1 = set,
            11 => self.r0 = set,
            12 => self.r1 = set,
            13 => self.m0 = set,
            14 => self.m1 = set,
            15 => self.ty = set,
            _ => unreachable!("SAM strobe index {index} out of range"),
        }
    }

    /// Decode a CPU address to its target, per the plan's TY=0/TY=1 memory
    /// map table.
    pub fn map(&self, addr: u16) -> SAMTarget {
        // The vector mirror and $FF00+ fixed page win regardless of TY.
        if addr >= VECTOR_MIRROR_BASE {
            return SAMTarget::RomBas(BAS_MIRROR_OFFSET + (addr - VECTOR_MIRROR_BASE) as usize);
        }
        if addr >= STROBE_BASE {
            return SAMTarget::Io; // $FFC0-$FFDF: SAM control strobes.
        }
        if (OPEN_BUS_BASE..=OPEN_BUS_LAST).contains(&addr) {
            return SAMTarget::OpenBus; // $FF7F-$FFBF.
        }
        if addr >= IO_BASE {
            return SAMTarget::Io; // $FF00-$FF7E: PIA0/PIA1/cart SCS (+ extension).
        }
        if self.ty && self.is_64k() {
            // All-RAM mode extends the RAM decode through $FEFF.
            return SAMTarget::Ram(addr as usize);
        }
        match addr {
            0x0000..=0x7FFF => {
                // P1 only matters when TY=0 (this branch) and 64K: ORs $8000 into the RAM address.
                let ram_addr = if self.p1 && self.is_64k() {
                    addr | 0x8000
                } else {
                    addr
                };
                SAMTarget::Ram(ram_addr as usize)
            }
            EXT_ROM_BASE..=EXT_ROM_LAST => SAMTarget::RomExt((addr - EXT_ROM_BASE) as usize),
            BAS_ROM_BASE..=BAS_ROM_LAST => SAMTarget::RomBas((addr - BAS_ROM_BASE) as usize),
            CART_ROM_BASE..=CART_ROM_LAST => SAMTarget::Cart((addr - CART_ROM_BASE) as usize),
            _ => unreachable!("address {addr:#06x} not covered by the SAM decode"),
        }
    }

    /// CoCo-compatible video base address: the F-bits times [`PAGE_UNIT`].
    pub fn display_base(&self) -> usize {
        self.f as usize * PAGE_UNIT
    }

    /// The V0-V2 VDG-counter mode bits, packed as `V2:V1:V0` (0-7).
    pub fn v_bits(&self) -> u8 {
        self.v
    }

    /// True when either CPU-rate strobe selects double speed. KNOWN GAP: real
    /// hardware's R0 is address-dependent (ROM fast/RAM slow); unmodeled, matching MAME.
    pub fn cpu_fast(&self) -> bool {
        self.r0 || self.r1
    }

    /// The 64K/"not 64K" shorthand (`M1` set), gating TY's all-RAM mode and P1
    /// banking. KNOWN GAP: full M0:M1 4K/16K/32K/64K decode isn't modeled.
    fn is_64k(&self) -> bool {
        self.m1
    }
}

/// Set or clear `bit` of `reg` (even/odd-strobe convention shared by V/F).
fn set_bit(reg: &mut u8, bit: u8, set: bool) {
    let mask = 1u8 << bit;
    if set {
        *reg |= mask;
    } else {
        *reg &= !mask;
    }
}
