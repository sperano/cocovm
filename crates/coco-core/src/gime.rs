//! GIME — MMU + video + timer + interrupt controller. See `DESIGN.md` §3, §4, §6.
//!
//! STATUS: skeleton. Register storage and a basic MMU translate are present;
//! ROM mapping, the disabled-MMU power-on map, the write-8/read-6 register
//! asymmetry, real video scanout, and interrupt generation are TODO.

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub palette: [u8; PALETTE_LEN],
    pub border: u8,
    pub vmode: u8,
    pub vres: u8,
    pub vertical_offset: u16,
    pub horizontal_offset: u8,
    /// 12-bit timer reload value and live count.
    pub timer_reload: u16,
    pub timer_count: u16,
    pub irq_enable: u8,
    pub firq_enable: u8,
    pub irq_pending: u8,
    pub firq_pending: u8,
}

impl Default for GIME {
    fn default() -> Self {
        Self {
            mmu: [[0; SLOTS_PER_TASK]; TASK_COUNT],
            task: 0,
            mmu_enabled: false,
            init0: 0,
            init1: 0,
            all_ram: false,
            palette: [0; PALETTE_LEN],
            border: 0,
            vmode: 0,
            vres: 0,
            vertical_offset: 0,
            horizontal_offset: 0,
            timer_reload: 0,
            timer_count: 0,
            irq_enable: 0,
            firq_enable: 0,
            irq_pending: 0,
            firq_pending: 0,
        }
    }
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

    /// Advance the 12-bit timer by `cycles`; returns true on underflow (reload).
    /// Skeleton: no interrupt raised yet (`DESIGN.md` §4).
    pub fn tick_timer(&mut self, _cycles: u32) -> bool {
        false
    }
}
