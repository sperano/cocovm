//! GIME — MMU + video + timer + interrupt controller. See `DESIGN.md` §3, §4, §6.
//!
//! STATUS: MMU translate, SAM compatibility strobes, and the video registers
//! ($FF98–$FF9F) are modelled; native scanout lives in `gime_video`. The timer,
//! GIME-sourced interrupts, and the write-8/read-6 register asymmetry are TODO.

use serde::{Deserialize, Serialize};

use crate::config::BLOCK_SIZE;

/// Number of MMU task register sets ($FFA0–A7 and $FFA8–AF).
pub const TASK_COUNT: usize = 2;
/// Logical 8K slots per task (the 64K CPU space / 8K).
pub const SLOTS_PER_TASK: usize = 8;
/// GIME palette register count ($FFB0–$FFBF).
pub const PALETTE_LEN: usize = 16;

/// log2 of the 8K MMU block size — an 8K logical slot / physical block.
const BLOCK_SHIFT: u32 = 13;

/// Physical base of the 64K window used when the MMU is disabled.
///
/// SEB Unravelled II: with MMUEN clear the logical 64K is the contiguous physical
/// segment `$70000–$7FFFF` (the CoCo-1/2-compatible top-of-512K region). `SystemBus`
/// masks the result to installed RAM, which naturally relocates smaller machines
/// into their high blocks. See `DESIGN.md` §3.
pub const DISABLED_MMU_BASE: usize = 0x7_0000;

/// MMU register readback mask — only the low 6 bits read back reliably; the upper
/// two are bus bleedover on most machines (SEB Unravelled II, MMU special note 1).
pub const MMU_READ_MASK: u8 = 0x3F;

/// SAM-compatibility control strobes ($FFC0–$FFDF). Each SAM bit is a pair of
/// addresses: the even one clears it, the odd sets it (the data written is
/// ignored). See `DESIGN.md` §3.
pub const SAM_BASE: u16 = 0xFFC0;
pub const SAM_LAST: u16 = 0xFFDF;
/// VDG-mode strobe pairs V0–V2 ($FFC0–$FFC5): 3 bits selecting the legacy
/// (CoCo-compatible) graphics vertical row cadence — see
/// [`crate::video::LEGACY_GFX_LINES_PER_ROW`]. Latched unconditionally, but
/// only *used* when INIT0 COCO=1 (SEB Unravelled II; MAME `6883sam.cpp`).
/// Even address clears a bit, odd sets it, same as F0–F6.
pub const SAM_VDG_BASE: u16 = 0xFFC0;
pub const SAM_VDG_LAST: u16 = 0xFFC5;
/// TY (map type) strobe pair — the highest SAM bit. `$FFDE` clears TY (system ROM
/// mapped in the `$8000–$FFFF` window); `$FFDF` sets TY (all-RAM: the ROM is
/// switched out and the RAM underneath — into which BASIC copies and *patches* a
/// working image of itself — becomes visible). The CoCo 3 runs BASIC from this
/// patched RAM copy; the Super Extended init depends on the switch (SEB Unravelled II).
pub const SAM_TY_CLEAR: u16 = 0xFFDE;
pub const SAM_TY_SET: u16 = 0xFFDF;
/// Page-select strobe pairs F0–F6 ($FFC6–$FFD3): 7 bits selecting the video base in
/// units of [`SAM_PAGE_UNIT`]. Even address clears a bit, odd sets it.
pub const SAM_PAGE_BASE: u16 = 0xFFC6;
pub const SAM_PAGE_LAST: u16 = 0xFFD3;
/// R1 CPU-rate strobe pair: $FFD9 switches to true double speed (~1.79 MHz),
/// $FFD8 back to ~0.89 MHz — the classic `POKE 65497,0` / `POKE 65496,0`.
/// The CoCo 1/2 R0 pair ($FFD6/$FFD7, address-dependent speed) is inert on the
/// CoCo 3 — SEB Unravelled II Fig 8 lists only R1 as active.
pub const SAM_R1_CLEAR: u16 = 0xFFD8;
pub const SAM_R1_SET: u16 = 0xFFD9;
/// Each page-select step is 512 bytes (base = `sam_page * SAM_PAGE_UNIT`).
pub const SAM_PAGE_UNIT: u16 = 512;

/// INIT0 ($FF90) bit assignments (SEB Unravelled II).
pub mod init0 {
    /// 1 = CoCo 1/2 compatible mode (enables SAM video/offset regs).
    pub const COCO: u8 = 0x80;
    /// 1 = MMU enabled.
    pub const MMUEN: u8 = 0x40;
    /// 1 = GIME IRQ structure enabled (0 = legacy PIA IRQ path).
    pub const IEN: u8 = 0x20;
    /// 1 = GIME FIRQ structure enabled (0 = legacy PIA FIRQ path).
    pub const FEN: u8 = 0x10;
    /// 1 = $FE00–$FEFF held constant at $7FE00–$7FEFF regardless of the MMU.
    pub const MC3: u8 = 0x08;
    /// Spare chip-select ($SCS) width control.
    pub const MC2: u8 = 0x04;
    /// ROM map control, high bit.
    pub const MC1: u8 = 0x02;
    /// ROM map control, low bit.
    pub const MC0: u8 = 0x01;
}

/// INIT1 ($FF91) bit assignments (SEB Unravelled II).
pub mod init1 {
    /// Timer input select: 1 = ~70 ns (14.318 MHz), 0 = ~63.5 µs (horizontal rate).
    pub const TINS: u8 = 0x20;
    /// Task register select: 0 = $FFA0 set, 1 = $FFA8 set.
    pub const TR: u8 = 0x01;
}

/// Video Mode Register ($FF98) bit assignments (SEB Unravelled II). Only meaningful
/// when INIT0 COCO=0 (GIME native modes); ignored in CoCo-compatible mode.
pub mod vmode {
    /// Bit-plane / graphics select: 1 = graphics (HSCREEN), 0 = hi-res text.
    pub const BP: u8 = 0x80;
    /// Burst phase invert (alternate composite colour set).
    pub const BPI: u8 = 0x20;
    /// Monochrome on composite output.
    pub const MOCH: u8 = 0x10;
    /// 50 Hz field rate (else 60 Hz).
    pub const H50: u8 = 0x08;
    /// Lines per character row (text modes).
    pub const LPR_MASK: u8 = 0x07;
}

/// Video Resolution Register ($FF99) bit assignments (SEB Unravelled II): rows per
/// field (LPF), bytes per row (HRES), and colour depth (CRES).
pub mod vres {
    /// Lines-per-field select (bits 5–6): 192/200/210/225 rows.
    pub const LPF_MASK: u8 = 0x60;
    pub const LPF_SHIFT: u8 = 5;
    /// Horizontal resolution select (bits 2–4): sets bytes per row, not pixels.
    pub const HRES_MASK: u8 = 0x1C;
    pub const HRES_SHIFT: u8 = 2;
    /// Colour-resolution select (bits 0–1): pixels packed per byte (2/4/16 colours).
    /// In text modes (BP=0) bit 0 instead enables per-character attribute bytes.
    pub const CRES_MASK: u8 = 0x03;
    /// Text-mode attribute enable (CRES bit 0, BP=0 only).
    pub const TEXT_ATTR: u8 = 0x01;
}

/// Horizontal Offset Register ($FF9F) bit assignments (SEB Unravelled II).
pub mod hoff {
    /// Horizontal virtual enable: rows are 256 bytes wide; the display is a
    /// scrollable window into them.
    pub const HVEN: u8 = 0x80;
    /// X0–X6 horizontal offset; ×2 gives the byte offset added within each row.
    pub const X_MASK: u8 = 0x7F;
}

/// Active display lines per field, indexed by the VRES LPF field.
///
/// LPF=%10 is documented as 210 lines (SEB Unravelled II); on real hardware it
/// is a glitched "infinite" count (MAME `update_geometry`) — 210 is the sane
/// approximation.
pub const LPF_LINES: [usize; 4] = [192, 200, 210, 225];

/// Lines per character row, indexed by the $FF98 LPR field. Hardware-verified
/// values from MAME `get_lines_per_row` (SEB's table says 1/2/3/8/9/10/12 but
/// the chip does 1/1/2/8/9/10/11; LPR=%111 repeats one glitched line forever,
/// approximated by a huge count so only the first row ever shows).
pub const LPR_LINES: [usize; 8] = [1, 1, 2, 8, 9, 10, 11, usize::MAX];

/// Text columns per row, indexed by the VRES HRES field (BP=0). HRES bit 1 is
/// ignored by the chip in text modes (MAME dispatches on $FF99 & $15), which
/// yields SEB's 32/40/32/40/64/80/64/80 table.
pub const TEXT_COLS: [usize; 8] = [32, 40, 32, 40, 64, 80, 64, 80];

/// Graphics bytes fetched per row, indexed by the VRES HRES field (BP=1).
pub const GFX_BYTES_PER_ROW: [usize; 8] = [16, 20, 32, 40, 64, 80, 128, 160];

/// Graphics bits per pixel, indexed by the VRES CRES field (BP=1): 2, 4, or 16
/// colours. CRES=%11 is undefined on the GIME; 4 bpp is the closest behaviour.
pub const GFX_BPP: [usize; 4] = [1, 2, 4, 4];

/// Virtual row width in bytes when $FF9F HVEN is set.
pub const HVEN_ROW_BYTES: usize = 256;

/// Start of the upper ROM half — external (cartridge, CTS*) when INIT0
/// MC1:MC0 selects a 16K+16K map.
pub const EXTERNAL_ROM_BASE: u16 = 0xC000;

/// Interrupt source bits shared by IRQENR ($FF92) and FIRQENR ($FF93)
/// (SEB Unravelled II Fig 14). Write = per-source enable; read = latched
/// status, cleared by the read.
pub mod intr {
    /// Timer interrupt: the 12-bit interval timer counted down through zero.
    pub const TMR: u8 = 0x20;
    /// Horizontal border: falling edge of horizontal sync, once per scanline.
    pub const HBORD: u8 = 0x10;
    /// Vertical border: falling edge of vertical sync, once per field.
    pub const VBORD: u8 = 0x08;
    /// Serial data: falling edge on the serial connector status pin.
    pub const EI2: u8 = 0x04;
    /// Keyboard: a zero appearing on any PIA0 PA0–PA6 row sense line.
    pub const EI1: u8 = 0x02;
    /// Cartridge: falling edge on the expansion connector CART pin.
    pub const EI0: u8 = 0x01;
    /// Bits 6–7 are unused; enables are masked to the six real sources.
    pub const SOURCE_MASK: u8 = 0x3F;
}

/// The 12-bit timer counts this many extra ticks past the programmed value on
/// each (re)load. Hardware-measured on the 1986 GIME (MAME `gime.cpp`
/// `reset_timer`); the 1987 revision uses +1. We model the 1986 chip.
pub const TIMER_RELOAD_OFFSET: u16 = 2;
/// Programmed timer values are 12 bits ($FF94 low nibble + $FF95).
pub const TIMER_VALUE_MASK: u16 = 0x0FFF;

/// Which monitor signal path resolves 6-bit palette values to RGB: the real
/// GIME drives both an RGB and a composite output simultaneously, and it's
/// the monitor cable — not a GIME register — that decides which one matters.
/// Emulator config/UI choice; default matches prior (RGB-only) behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MonitorType {
    #[default]
    Rgb,
    Composite,
}

/// Composite-monitor palette (BPI=0), 64 entries indexed by the 6-bit GIME
/// palette value, `0xRRGGBB`. Hand-measured on real hardware — there is no
/// formula. Verbatim from MAME `src/mame/trs/gime.cpp` `get_composite_color`
/// (BSD-3-Clause, Nathan Woods; see NOTICE.md).
const COMPOSITE_PALETTE: [u32; 64] = [
    0x000000, 0x004c00, 0x004300, 0x0a3100, 0x2f1b00, 0x550100, 0x6c0000, 0x770006, 0x71004b,
    0x5c008b, 0x3b00b8, 0x1100ca, 0x001499, 0x002c62, 0x004011, 0x004b00, 0x2d2d2d, 0x069800,
    0x288f00, 0x537d00, 0x786700, 0xa04c00, 0xb63402, 0xc3224c, 0xbd1693, 0xa814d5, 0x881cfe,
    0x5e2cff, 0x105ee9, 0x0076b2, 0x008b60, 0x009618, 0x747474, 0x41d714, 0x62cf00, 0x8ebd00,
    0xb4a700, 0xdd8c01, 0xf5733a, 0xfe6085, 0xfd53ce, 0xe950ff, 0xc958ff, 0x9e67ff, 0x4e9aff,
    0x36b3f7, 0x26c9a3, 0x2bd558, 0xfdfdfe, 0x88e85a, 0xa1e03f, 0xbed238, 0xd8c342, 0xf1b161,
    0xfea08d, 0xfe95bf, 0xfd8ef1, 0xef8eff, 0xd895ff, 0xb9a1ff, 0x86c4ff, 0x78d4f2, 0x71e2b6,
    0xffffff,
];
/// Composite-monitor palette with $FF98 BPI (burst phase invert) set.
/// Verbatim from MAME `gime.cpp` `get_composite_color`.
const COMPOSITE_PALETTE_180: [u32; 64] = [
    0x000000, 0x5a0e5a, 0x4f0c4f, 0x360f40, 0x0d213c, 0x003334, 0x004141, 0x004943, 0x005409,
    0x005600, 0x114c00, 0x263700, 0x392500, 0x491d00, 0x4f0f3e, 0x590e59, 0x2d2d2d, 0xb11fb7,
    0x9932c1, 0x7248c5, 0x4a5bc2, 0x1a6eba, 0x0077a9, 0x008c62, 0x009619, 0x039700, 0x238f00,
    0x467800, 0x9c4e00, 0xb23c00, 0xb92e59, 0xb6209e, 0x747474, 0xe852ff, 0xcd60ff, 0xa677ff,
    0x7d8aff, 0x4d9eff, 0x32b4ed, 0x29c7a2, 0x2ad459, 0x39d223, 0x50c11a, 0x72a911, 0xcf831e,
    0xf47733, 0xff5f85, 0xfe54d1, 0xfdfdfc, 0xef8fff, 0xd697ff, 0xb8a4ff, 0x9eb3ff, 0x86c6ff,
    0x76d4e7, 0x74ddb3, 0x77e683, 0x80e170, 0x92d56b, 0xacc466, 0xeaac71, 0xffa385, 0xff95c1,
    0xffffff,
];

/// Unpack an `0xRRGGBB` composite-table entry into RGBA, matching
/// [`GIME::rgb_color`]'s return convention (opaque alpha).
fn unpack_rgb(v: u32) -> [u8; 4] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8, 0xFF]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GIME {
    /// MMU task registers: `[task][logical 8K slot]` -> physical block number.
    /// Writes store the full 8 bits; reads return only the low 6 reliably
    /// (`DESIGN.md` §3 — read path not yet modelled).
    pub mmu: [[u8; SLOTS_PER_TASK]; TASK_COUNT],
    pub task: usize,
    pub mmu_enabled: bool,
    /// Raw INIT0 ($FF90): COCO/MMUEN/IEN/FEN/MC3/MC2/MC1/MC0.
    pub init0: u8,
    /// Raw INIT1 ($FF91): TINS/TR.
    pub init1: u8,
    /// SAM map-type bit ($FFDE/$FFDF, TY): true = all-RAM (system ROM disabled).
    pub all_ram: bool,
    /// SAM display page-select bits F0–F6 ($FFC6–$FFD3). The CoCo-compatible video
    /// base is `sam_page * SAM_PAGE_UNIT` (`DESIGN.md` §6).
    pub sam_page: u8,
    /// SAM VDG-mode bits V0–V2 ($FFC0–$FFC5), packed as `V2:V1:V0` (0–7).
    /// Latched always; only consulted in CoCo-compatible mode (INIT0 COCO=1),
    /// where it sets the legacy graphics vertical row cadence — see
    /// [`crate::video::LEGACY_GFX_LINES_PER_ROW`]. The horizontal decode
    /// (bytes/row, bpp, colour set) stays PIA1 $FF22 GM/CSS-derived.
    pub sam_video: u8,
    pub palette: [u8; PALETTE_LEN],
    /// Border colour register ($FF9A): a 6-bit colour value (not a palette index).
    pub border: u8,
    pub vmode: u8,
    pub vres: u8,
    /// $FF9B: 512K video-bank select for >512K machines (low bits become physical
    /// address bits 19+; unused on stock 128K/512K). MAME `record_scanline_res`.
    pub video_bank: u8,
    /// Vertical scroll register ($FF9C) low nibble: the character-row line the
    /// field starts on, for smooth text scrolling.
    pub vertical_scroll: u8,
    /// Vertical offset registers $FF9D (high byte) / $FF9E (low byte). GIME-native
    /// video starts at physical `vertical_offset * 8`.
    pub vertical_offset: u16,
    /// Horizontal offset register ($FF9F): HVEN + X offset (see [`hoff`]).
    pub horizontal_offset: u8,
    /// 12-bit timer programmed value ($FF94 low nibble / $FF95). Zero inhibits
    /// the count; nonzero reloads (+[`TIMER_RELOAD_OFFSET`]) on each underflow.
    pub timer_reload: u16,
    /// Live countdown, in timer input clocks (INIT1 TINS selects the rate).
    pub timer_count: u16,
    /// IRQENR ($FF92) / FIRQENR ($FF93) source enables (see [`intr`]).
    pub irq_enable: u8,
    pub firq_enable: u8,
    /// Latched interrupt status: sources that fired while enabled. Cleared by
    /// reading the register (or by writing 0 to the source's enable bit).
    pub irq_pending: u8,
    pub firq_pending: u8,
    /// Text-attribute blink phase; toggles on every timer underflow.
    pub blink_state: bool,
    /// SAM R1 CPU-rate bit ($FFD8/$FFD9): true = double speed (~1.79 MHz).
    pub cpu_fast: bool,
    /// Which monitor cable is plugged in ([`MonitorType`]). Not a hardware
    /// register — the real GIME always drives both RGB and composite outputs;
    /// this is purely an emulator display choice.
    pub monitor: MonitorType,
}

impl GIME {
    pub fn new() -> Self {
        Self::default()
    }

    /// Translate a CPU logical address to a physical RAM offset.
    ///
    /// With the MMU enabled the active task's 8K block maps each logical slot;
    /// with it disabled the whole 64K sits at the fixed `$70000` window (SEB
    /// Unravelled II). The caller masks the result to installed RAM. ROM overlay
    /// and the `$FE00–$FEFF` MC3 constant page are handled in `SystemBus`.
    pub fn translate(&self, addr: u16) -> usize {
        if self.mmu_enabled {
            let slot = (addr as usize >> BLOCK_SHIFT) & (SLOTS_PER_TASK - 1);
            let block = self.mmu[self.task][slot] as usize;
            (block << BLOCK_SHIFT) | (addr as usize & (BLOCK_SIZE - 1))
        } else {
            DISABLED_MMU_BASE | (addr as usize)
        }
    }

    /// Apply a write to INIT0 ($FF90): latch it and derive MMU-enable.
    pub fn write_init0(&mut self, val: u8) {
        self.init0 = val;
        self.mmu_enabled = val & init0::MMUEN != 0;
    }

    /// Apply a write to INIT1 ($FF91): latch it and derive the active task set.
    pub fn write_init1(&mut self, val: u8) {
        self.init1 = val;
        self.task = usize::from(val & init1::TR != 0);
    }

    /// Apply a SAM control-register strobe ($FFC0–$FFDF). V0–V2 ($FFC0–$FFC5)
    /// select the CoCo-compatible legacy-graphics vertical cadence, F0–F6
    /// ($FFC6–$FFD3) the CoCo-compatible video base, R1 ($FFD8/$FFD9) the CPU
    /// rate, and TY ($FFDE/$FFDF) selects the all-RAM map. Not modelled: the
    /// inert-on-CoCo-3 R0 pair, and P1/M0/M1.
    pub fn write_sam(&mut self, addr: u16) {
        match addr {
            SAM_VDG_BASE..=SAM_VDG_LAST => {
                let bit = (addr - SAM_VDG_BASE) / 2;
                let mask = 1u8 << bit;
                if (addr - SAM_VDG_BASE) & 1 == 0 {
                    self.sam_video &= !mask; // even address clears the bit
                } else {
                    self.sam_video |= mask; // odd address sets the bit
                }
            }
            SAM_TY_CLEAR => self.all_ram = false,
            SAM_TY_SET => self.all_ram = true,
            SAM_R1_CLEAR => self.cpu_fast = false,
            SAM_R1_SET => self.cpu_fast = true,
            SAM_PAGE_BASE..=SAM_PAGE_LAST => {
                let bit = (addr - SAM_PAGE_BASE) / 2;
                let mask = 1u8 << bit;
                if (addr - SAM_PAGE_BASE) & 1 == 0 {
                    self.sam_page &= !mask; // even address clears the bit
                } else {
                    self.sam_page |= mask; // odd address sets the bit
                }
            }
            _ => {}
        }
    }

    /// CoCo-compatible video base address: the SAM page bits times 512.
    pub fn sam_display_base(&self) -> u16 {
        (self.sam_page as u16).wrapping_mul(SAM_PAGE_UNIT)
    }

    /// Physical start address of the GIME-native video display: the vertical
    /// offset registers ×8 (any 8-byte boundary in the 512K space), plus the
    /// $FF9B 512K bank on >512K machines. GIME-native scanout bypasses the MMU
    /// entirely — this is a physical address (SEB Unravelled II Fig 6).
    pub fn video_base(&self) -> usize {
        ((self.video_bank as usize & 0x0F) << 19) | ((self.vertical_offset as usize) << 3)
    }

    /// Lines per character row from the $FF98 LPR field (also applied to
    /// graphics rows, where BASIC's HSCREEN setup selects 1).
    pub fn lines_per_row(&self) -> usize {
        LPR_LINES[(self.vmode & vmode::LPR_MASK) as usize]
    }

    /// Active display lines in the current field from the VRES LPF bits.
    pub fn lines_per_field(&self) -> usize {
        LPF_LINES[((self.vres & vres::LPF_MASK) >> vres::LPF_SHIFT) as usize]
    }

    /// True when the system ROM is visible in the `$8000–$FFFF` window
    /// (SAM map-type = ROM). When all-RAM is selected the region is plain RAM.
    pub fn rom_enabled(&self) -> bool {
        !self.all_ram
    }

    /// True when `addr` in the ROM window maps to the *external* (cartridge)
    /// ROM, per INIT0 MC1:MC0 (SEB Unravelled II ROM-map table):
    /// `00`/`01` = 16K internal + 16K external at `$C000`; `10` = 32K
    /// internal; `11` = 32K external (the CPU vectors stay internal — the bus
    /// handles those separately). The cold-start writes INIT0 with MC=`10`
    /// before its `JMP $C000`, which is why a diskless boot runs internal ROM.
    pub fn rom_is_external(&self, addr: u16) -> bool {
        match self.init0 & (init0::MC1 | init0::MC0) {
            0b10 => false,
            0b11 => true,
            _ => addr >= EXTERNAL_ROM_BASE,
        }
    }

    /// Convert a 6-bit GIME palette value to RGBA. The register format is
    /// `RGBrgb` (two bits per channel); each channel scales `0..3` to `0..0xFF`
    /// via `×0x55` — matching the GIME's RGB output (MAME `gime.cpp`).
    pub fn rgb_color(value: u8) -> [u8; 4] {
        let chan = |hi_bit: u8, lo_bit: u8| {
            let v = ((value >> hi_bit) & 1) << 1 | ((value >> lo_bit) & 1);
            v * 0x55
        };
        [chan(5, 2), chan(4, 1), chan(3, 0), 0xFF]
    }

    /// Resolve a 6-bit GIME palette value to RGBA through the currently
    /// selected monitor path (`self.monitor`). RGB mode is [`Self::rgb_color`]
    /// unconditionally; composite mode picks [`COMPOSITE_PALETTE`] or
    /// [`COMPOSITE_PALETTE_180`] per $FF98 BPI, then averages channels to
    /// grey when $FF98 MOCH is set (MAME `gime.cpp` `update_composite`).
    pub fn color(&self, value: u8) -> [u8; 4] {
        match self.monitor {
            MonitorType::Rgb => Self::rgb_color(value),
            MonitorType::Composite => {
                let table = if self.vmode & vmode::BPI != 0 {
                    &COMPOSITE_PALETTE_180
                } else {
                    &COMPOSITE_PALETTE
                };
                let [r, g, b, a] = unpack_rgb(table[value as usize & 0x3F]);
                if self.vmode & vmode::MOCH != 0 {
                    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                    [avg, avg, avg, a]
                } else {
                    [r, g, b, a]
                }
            }
        }
    }

    /// Write IRQENR ($FF92): set the per-source IRQ enables. Writing 0 to an
    /// enable bit also clears that source's latched status — a hardware anomaly
    /// SEB Unravelled II documents and MAME models (`change_gime_irq(m_irq & data)`).
    pub fn write_irq_enable(&mut self, val: u8) {
        self.irq_pending &= val;
        self.irq_enable = val & intr::SOURCE_MASK;
    }

    /// Write FIRQENR ($FF93): the FIRQ twin of [`Self::write_irq_enable`].
    pub fn write_firq_enable(&mut self, val: u8) {
        self.firq_pending &= val;
        self.firq_enable = val & intr::SOURCE_MASK;
    }

    /// Read IRQENR ($FF92): returns the latched IRQ status and clears it
    /// (SEB Unravelled II — reading the status register resets the flags).
    pub fn read_irq_status(&mut self) -> u8 {
        std::mem::take(&mut self.irq_pending)
    }

    /// Read FIRQENR ($FF93): returns the latched FIRQ status and clears it.
    pub fn read_firq_status(&mut self) -> u8 {
        std::mem::take(&mut self.firq_pending)
    }

    /// Signal an interrupt source edge (an [`intr`] bit). The source latches
    /// into the IRQ/FIRQ status only where its enable bit is set — GIME
    /// interrupts trigger "when the enable line is high" (SEB Unravelled II).
    pub fn raise(&mut self, source: u8) {
        self.irq_pending |= source & self.irq_enable;
        self.firq_pending |= source & self.firq_enable;
    }

    /// True while the GIME holds the CPU IRQ line: any latched source, gated
    /// by the INIT0 IEN master enable.
    pub fn irq_asserted(&self) -> bool {
        self.init0 & init0::IEN != 0 && self.irq_pending != 0
    }

    /// True while the GIME holds the CPU FIRQ line (INIT0 FEN master enable).
    pub fn firq_asserted(&self) -> bool {
        self.init0 & init0::FEN != 0 && self.firq_pending != 0
    }

    /// Write the timer MSB ($FF94, low nibble = timer bits 8–11) and restart
    /// the count. SEB documents the MSB write as starting the timer; on the
    /// real chip either byte restarts it (MAME `reset_timer` on both).
    pub fn write_timer_msb(&mut self, val: u8) {
        self.timer_reload =
            (self.timer_reload & 0x00FF) | (u16::from(val) << 8 & TIMER_VALUE_MASK);
        self.restart_timer();
    }

    /// Write the timer LSB ($FF95) and restart the count.
    pub fn write_timer_lsb(&mut self, val: u8) {
        self.timer_reload = (self.timer_reload & 0x0F00) | u16::from(val);
        self.restart_timer();
    }

    /// Reload the live count from the programmed value. A zero value inhibits
    /// the countdown; nonzero counts value + [`TIMER_RELOAD_OFFSET`] input
    /// clocks per period (1986 GIME behaviour).
    fn restart_timer(&mut self) {
        self.timer_count = if self.timer_reload == 0 {
            0
        } else {
            self.timer_reload + TIMER_RELOAD_OFFSET
        };
    }

    /// True when INIT1 TINS selects the fast (3.58 MHz-class) timer clock
    /// rather than the horizontal-sync rate.
    pub fn timer_is_fast(&self) -> bool {
        self.init1 & init1::TINS != 0
    }

    /// Advance the 12-bit timer by `ticks` input clocks. Each underflow raises
    /// the TMR interrupt source, toggles the text blink phase, and reloads
    /// (SEB Unravelled II; MAME `timer_elapsed`). Inhibited while the
    /// programmed value is zero.
    pub fn tick_timer(&mut self, ticks: u32) {
        if self.timer_reload == 0 {
            return;
        }
        let mut remaining = ticks;
        while remaining >= u32::from(self.timer_count) {
            remaining -= u32::from(self.timer_count);
            self.blink_state = !self.blink_state;
            self.raise(intr::TMR);
            self.restart_timer();
        }
        self.timer_count -= remaining as u16;
    }
}
