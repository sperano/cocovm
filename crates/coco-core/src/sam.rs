//! `SAM` — the MC6883 Synchronous Address Multiplexer's primary memory map, for
//! the plain CoCo 1/2 machine (no GIME).
//!
//! This is a standalone model, deliberately **not** shared with the GIME's own
//! SAM-compatibility overlay (`gime.rs::write_sam`/`SAM_BASE`..): the GIME keeps
//! modelling its own CoCo-3-compatible strobes independently, and this file
//! duplicates the handful of constants that describe (verified against MAME
//! `6883sam.cpp`, the CoCo 2 NTSC Service Manual SAM register map, pages 8–10,
//! and Bob Russell's memory map. This small duplication is intentional: it
//! keeps the CoCo 3 path completely untouched.

use serde::{Deserialize, Serialize};

use crate::cart::{COCOMAX_IO_BASE, COCOMAX_IO_LAST};

/// Base/last of the SAM control-strobe address range ($FFC0–$FFDF). Every SAM
/// bit is a pair of addresses: the even one clears it, the odd one sets it —
/// the data written is ignored (service manual p. 8; MAME `6883sam.h`
/// `alter_sam_state`).
pub const STROBE_BASE: u16 = 0xFFC0;
pub const STROBE_LAST: u16 = 0xFFDF;

/// $FF00–$FF7E: PIA0, PIA1, cart SCS* ($FF40–$FF5F), and the cart SCS*
/// extension some cartridges decode ($FF60–$FF7E, such as the Sound/Speech
/// Cartridge's $FF7D/$FF7E — wiki `cocovm/cartridges` "Carts can decode
/// addresses outside SCS") — decoded by the bus, not `SAM` itself (`SAM::map`
/// only reports that this range is I/O).
const IO_BASE: u16 = 0xFF00;
/// $FF7F–$FFBF: no GIME (hence no MPI-style `$FF7F` decode either) on these
/// machines — open bus, apart from the [`COCOMAX_IO_BASE`]..[`COCOMAX_IO_LAST`]
/// carve-out below (real CoCo 1/2 hardware has nothing else in that range,
/// but a plugged-in CoCo Max module answers it directly off the raw address
/// bus, the same way the cart SCS* extension does).
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

/// MC6883 display-counter X divisors, indexed by V2:V1:V0.
const VIDEO_X_DIVISION: [u8; 8] = [1, 3, 1, 2, 1, 1, 1, 1];
/// MC6883 display-counter Y divisors, indexed by V2:V1:V0.
const VIDEO_Y_DIVISION: [u8; 8] = [12, 1, 3, 1, 2, 1, 1, 1];
const VIDEO_MODE_MASK: u8 = 0x07;
const VIDEO_MODE_LOW_BIT: u8 = 0x01;
const VIDEO_MODE_DMA: u8 = 0x07;
const COUNTER_DA0: u16 = 0x0001;
const COUNTER_B3: u16 = 0x0008;
const COUNTER_B4: u16 = 0x0010;
const COUNTER_LOW_NIBBLE: u16 = 0x000F;
const COUNTER_LOW_FIVE: u16 = 0x001F;
const COUNTER_ROW_INCREMENT: u16 = 0x0020;

/// One field's transient MC6883 video-address counter.
///
/// The MC6847 presents logical addresses, but the discrete SAM observes only
/// DA0 transitions. Its V bits independently select the divider and HS carry
/// rules that turn those transitions into physical RAM addresses. This state
/// is render-local because CoCo 1/2 fields are currently rendered from a
/// field-end snapshot; the CoCo 3 GIME path deliberately does not use it.
#[derive(Debug, Clone, Copy)]
pub struct SAMVideoAddressStream {
    counter: u16,
    x_phase: u8,
    y_phase: u8,
    video_mode: u8,
}

impl SAMVideoAddressStream {
    /// Reset the stream as the MC6847's asserted field sync does.
    pub fn new(display_base: u16, video_mode: u8) -> Self {
        Self {
            counter: display_base,
            x_phase: 0,
            y_phase: 0,
            video_mode: video_mode & VIDEO_MODE_MASK,
        }
    }

    /// Reload the F-derived base and clear both divider phases at field sync.
    pub fn reset(&mut self, display_base: u16) {
        self.counter = display_base;
        self.x_phase = 0;
        self.y_phase = 0;
    }

    /// Observe one MC6847 sample request and return the resulting SAM address.
    /// Only the logical address's DA0 level is visible to the MC6883.
    pub fn sample(&mut self, vdg_address: usize) -> u16 {
        if (vdg_address as u16 & COUNTER_DA0) != (self.counter & COUNTER_DA0) {
            self.increment_low_nibble();
        }
        self.counter
    }

    /// Apply the asserted edge of MC6847 horizontal sync.
    pub fn horizontal_sync(&mut self) {
        if self.video_mode == VIDEO_MODE_DMA {
            return;
        }
        if self.video_mode & VIDEO_MODE_LOW_BIT != 0 {
            self.clear_low_counter(COUNTER_LOW_NIBBLE, COUNTER_B3, true);
        } else {
            self.clear_low_counter(COUNTER_LOW_FIVE, COUNTER_B4, false);
        }
    }

    fn increment_low_nibble(&mut self) {
        let carry = self.counter & COUNTER_LOW_NIBBLE == COUNTER_LOW_NIBBLE;
        let next = self.counter.wrapping_add(1) & COUNTER_LOW_NIBBLE;
        self.counter = (self.counter & !COUNTER_LOW_NIBBLE) | next;
        if carry {
            self.carry_bit3();
        }
    }

    fn clear_low_counter(&mut self, clear_mask: u16, carry_bit: u16, carry_bit3: bool) {
        let carry = self.counter & carry_bit != 0;
        self.counter &= !clear_mask;
        if carry_bit3 && carry {
            self.carry_bit3();
        } else if carry {
            self.carry_bit4();
        }
    }

    fn carry_bit3(&mut self) {
        self.x_phase += 1;
        if self.x_phase < VIDEO_X_DIVISION[self.video_mode as usize] {
            return;
        }
        self.x_phase = 0;
        self.counter ^= COUNTER_B4;
        if self.counter & COUNTER_B4 == 0 {
            self.carry_bit4();
        }
    }

    fn carry_bit4(&mut self) {
        self.y_phase += 1;
        if self.y_phase >= VIDEO_Y_DIVISION[self.video_mode as usize] {
            self.y_phase = 0;
            self.counter = self.counter.wrapping_add(COUNTER_ROW_INCREMENT);
        }
    }
}

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
    /// $FF90–$FF97: the CoCo Max Hi-Res Input Module's ADC window — routed
    /// to the cartridge's [`crate::cart::Cartridge::upper_io_read`]/
    /// `upper_io_write`/`upper_io_peek`, not the ordinary SCS* dispatch.
    CartUpperIo,
    /// $FF7F–$FFBF (minus the [`SAMTarget::CartUpperIo`] carve-out): no
    /// GIME registers exist on these machines.
    OpenBus,
}

/// MC6883 SAM register state. All 16 bits
/// power up clear. Fields are `pub` (matching `gime::GIME`'s style) so tests
/// can inspect latched state directly, same as `sam_video.rs` does for the
/// GIME's compatibility overlay; [`SAM::map`]/[`SAM::display_base`]/
/// [`SAM::v_bits`]/[`SAM::video_address_mask`]/[`SAM::cpu_fast`] are the API
/// real callers use.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SAM {
    /// VDG-counter mode bits V0-V2, packed as `V2:V1:V0` (0-7). The field
    /// renderer passes these divider/carry rules to [`SAMVideoAddressStream`].
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
    /// Memory-size bit 0 (4K/16K/32K-64K, with `m1`); also selects the video
    /// counter's physical-address mask through [`SAM::video_address_mask`].
    pub m0: bool,
    /// Memory-size bit 1; used as the 64K/"not 64K" proxy (see
    /// [`SAM::is_64k`]) for TY/P1 and for the video counter's address mask.
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
        if (COCOMAX_IO_BASE..=COCOMAX_IO_LAST).contains(&addr) {
            return SAMTarget::CartUpperIo; // $FF90-$FF97.
        }
        if (OPEN_BUS_BASE..=OPEN_BUS_LAST).contains(&addr) {
            return SAMTarget::OpenBus; // $FF7F-$FFBF (minus the carve-out above).
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

    /// Mask applied to the MC6883 video counter before physical RAM access.
    pub fn video_address_mask(&self) -> u16 {
        match (self.m1, self.m0) {
            (true, _) => u16::MAX,
            (false, true) => 0x3FFF,
            (false, false) => 0x0FFF,
        }
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
