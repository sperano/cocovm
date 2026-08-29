//! OKI MSM5832 register map (the Disto 2-N-1's chip) as a view over the
//! shared [`MSM6242`] clock core. Layout per the MSM5832 datasheet and
//! NitrOS-9 `clock2_disto2.asm`: 0-5 seconds/minutes/hours digits as the
//! MSM6242, 6 weekday, 7-12 day/month/year digits (one higher than the
//! MSM6242's), no control registers. Hour-tens bit 3 is the 24-hour-mode
//! flag (writable, `SetTime` ORs `$08` in), bit 2 is PM in 12-hour mode;
//! day-tens bit 2 is the leap-year flag — on the chip a software-set bit,
//! here derived from the year (writes drop it). Unused 13-15 read `$0F`
//! (MAME `msm5832.cpp`). The chip's HOLD pin is not reachable through this
//! view, so `MSM6242::time()` and the register file always agree.

use super::msm6242::MSM6242;

/// What the unused registers 13-15 read as (MAME `msm5832.cpp`).
const UNUSED_READ: u8 = 0x0F;

const REG_H10: u8 = 5;
const REG_W: u8 = 6;
const REG_D10: u8 = 8;
const REG_Y10: u8 = 12;
/// Hour-tens bit 3: 24-hour mode.
const H10_24H: u8 = 0x08;
/// Day-tens bit 2: leap year.
const D10_LEAP: u8 = 0x04;
/// Hour-tens bits the MSM6242 core already provides (tens + PM).
const H10_CORE_MASK: u8 = 0x07;
const D10_TENS_MASK: u8 = 0x03;

/// The MSM6242 register holding MSM5832 register `reg`, if any.
fn core_reg(reg: u8) -> Option<u8> {
    match reg {
        0..=REG_H10 => Some(reg),
        REG_W => Some(MSM6242::WEEKDAY_REG),
        7..=REG_Y10 => Some(reg - 1),
        _ => None,
    }
}

pub(super) fn read(core: &mut MSM6242, reg: u8) -> u8 {
    let Some(core_reg) = core_reg(reg) else {
        return UNUSED_READ;
    };
    let val = core.read(core_reg);
    match reg {
        REG_H10 => (val & H10_CORE_MASK) | if core.is_24h() { H10_24H } else { 0 },
        REG_D10 => {
            let leap = super::is_leap_year(core.time().year);
            (val & D10_TENS_MASK) | if leap { D10_LEAP } else { 0 }
        }
        _ => val,
    }
}

pub(super) fn write(core: &mut MSM6242, reg: u8, val: u8) {
    let Some(core_reg) = core_reg(reg) else {
        return;
    };
    match reg {
        REG_H10 => {
            core.set_24h(val & H10_24H != 0);
            core.write(core_reg, val & H10_CORE_MASK);
        }
        REG_D10 => core.write(core_reg, val & D10_TENS_MASK),
        _ => core.write(core_reg, val),
    }
}
