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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GIME {
    /// MMU task registers: `[task][logical 8K slot]` -> physical block number.
    /// Writes store the full 8 bits; reads return only the low 6 reliably
    /// (`DESIGN.md` §3 — read path not yet modelled).
    pub mmu: [[u8; SLOTS_PER_TASK]; TASK_COUNT],
    pub task: usize,
    pub mmu_enabled: bool,
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
    /// Skeleton: applies the active task's MMU block when enabled, otherwise an
    /// identity fallback. ROM mapping and the real disabled-MMU power-on map are
    /// TODO (`DESIGN.md` §3). The caller masks the result to RAM size.
    pub fn translate(&self, addr: u16) -> usize {
        if self.mmu_enabled {
            let slot = (addr as usize >> 13) & (SLOTS_PER_TASK - 1);
            let block = self.mmu[self.task][slot] as usize;
            (block * BLOCK_SIZE) | (addr as usize & (BLOCK_SIZE - 1))
        } else {
            addr as usize
        }
    }

    /// Advance the 12-bit timer by `cycles`; returns true on underflow (reload).
    /// Skeleton: no interrupt raised yet (`DESIGN.md` §4).
    pub fn tick_timer(&mut self, _cycles: u32) -> bool {
        false
    }
}
