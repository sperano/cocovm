//! SAM-compatibility control strobes ($FFC0–$FFDF) and the CoCo-compatible
//! video base they select. See `DESIGN.md` §3.

use super::GIME;

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

impl GIME {
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
}
