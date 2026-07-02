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

    /// Apply a SAM control-register strobe ($FFC0–$FFDF). The TY map-type bit
    /// ($FFDE/$FFDF) selects the all-RAM map; the F0–F6 page-select pairs
    /// ($FFC6–$FFD3) set the CoCo-compatible video base. The remaining SAM bits
    /// (V0–V2 VDG mode, clock rate) are compatibility strobes not modelled — the VDG
    /// mode is taken from PIA1 $FF22 instead (`DESIGN.md` §3/§6 TODO).
    pub fn write_sam(&mut self, addr: u16) {
        match addr {
            SAM_TY_CLEAR => self.all_ram = false,
            SAM_TY_SET => self.all_ram = true,
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
