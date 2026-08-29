//! Disto Memory Expansion Bus real-time clock: an OKI MSM6242 RTC decoded in
//! the SCS window at `$FF50-$FF53` (MAME `meb_rtime.cpp` `disto_rtime_device`).
//!
//! Register interface (MAME, verified against both NitrOS-9 Disto drivers):
//! - `$FF50` read/write — MSM6242 data register, indexed by the address latch.
//! - `$FF51` write — address latch (NitrOS-9 `clock2_disto4.asm`, the 4-N-1).
//! - `$FF52`/`$FF53` write — *also* the address latch (`clock2_disto2.asm`,
//!   the 2-N-1, selects through `$FF52`). On the real card these two double as
//!   the Centronics printer strobe, which is not modeled here; reads return
//!   the printer BUSY line (never busy) in bit 7.
//!
//! The two NitrOS-9 drivers hardcode different select offsets because they
//! target different Disto combo cards; MAME's decode is a superset that
//! satisfies both, and so is this one.
//!
//! Time never comes from `std::time` (coco-core stays host-clock-free): the
//! frontend injects a [`TimeSource`] closure, and the chip tracks it through a
//! signed offset so NitrOS-9's `setime` (writes to the time registers, real
//! MSM6242 behavior; MAME drops these writes) sticks without the emulated
//! clock drifting when the machine is paused or the CPU runs double-speed.

use serde::{Deserialize, Serialize};

use crate::cart::{Cartridge, IO_OPEN_BUS};

/// A calendar timestamp fed to the RTC by the host frontend. Fields are plain
/// binary (not BCD); `year` is the full year, such as 2026. The chip exposes
/// only `year % 100`, but keeping the century lets register writes preserve
/// it. The weekday register is derived from the date, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RTCTime {
    pub year: i32,
    /// 1-12.
    pub month: u8,
    /// 1-31.
    pub day: u8,
    /// 0-23.
    pub hour: u8,
    /// 0-59.
    pub minute: u8,
    /// 0-59.
    pub second: u8,
}

/// Host clock injected into the RTC — typically "read the host's local time",
/// but tests inject fixed or hand-advanced closures for determinism.
pub type TimeSource = Box<dyn FnMut() -> RTCTime>;

// ---- Civil-calendar <-> seconds conversion ---------------------------------
// Days-from-civil / civil-from-days after Howard Hinnant's public-domain
// algorithms (proleptic Gregorian, epoch 1970-01-01).

const SECS_PER_DAY: i64 = 86_400;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_MIN: i64 = 60;
/// 1970-01-01 was a Thursday; with Sunday = 0 (the MSM6242 W convention MAME
/// uses: `day_of_week - 1` where MAME's day-of-week is 1-7 Sunday-first).
const EPOCH_WEEKDAY: i64 = 4;

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let m = i64::from(month);
    let d = i64::from(day);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11], March-first
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i32, m as u8, d as u8)
}

impl RTCTime {
    /// Seconds since the 1970-01-01 00:00:00 epoch (proleptic Gregorian).
    /// Out-of-range month/day are clamped rather than rejected, so a
    /// partially-written register file mid-`setime` still returns something.
    fn to_secs(self) -> i64 {
        let month = self.month.clamp(1, 12);
        let day = self.day.clamp(1, 31);
        days_from_civil(self.year, month, day) * SECS_PER_DAY
            + i64::from(self.hour) * SECS_PER_HOUR
            + i64::from(self.minute) * SECS_PER_MIN
            + i64::from(self.second)
    }

    fn from_secs(secs: i64) -> Self {
        let days = secs.div_euclid(SECS_PER_DAY);
        let tod = secs.rem_euclid(SECS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (tod / SECS_PER_HOUR) as u8,
            minute: (tod % SECS_PER_HOUR / SECS_PER_MIN) as u8,
            second: (tod % SECS_PER_MIN) as u8,
        }
    }

    /// Day of week, 0 = Sunday (the MSM6242 W register convention).
    fn weekday(self) -> u8 {
        let days = self.to_secs().div_euclid(SECS_PER_DAY);
        (days + EPOCH_WEEKDAY).rem_euclid(7) as u8
    }
}

// ---- MSM6242 register file (MAME `msm6242.cpp` enum) ------------------------

const REG_S1: u8 = 0;
const REG_S10: u8 = 1;
const REG_MI1: u8 = 2;
const REG_MI10: u8 = 3;
const REG_H1: u8 = 4;
const REG_H10: u8 = 5;
const REG_D1: u8 = 6;
const REG_D10: u8 = 7;
const REG_MO1: u8 = 8;
const REG_MO10: u8 = 9;
const REG_Y1: u8 = 10;
const REG_Y10: u8 = 11;
const REG_W: u8 = 12;
const REG_CD: u8 = 13;
const REG_CE: u8 = 14;
const REG_CF: u8 = 15;

/// CD (control D) bits.
mod cd {
    /// Freeze the register file for a consistent multi-nibble read/write.
    pub const HOLD: u8 = 0x01;
    /// Round to the nearest minute (write 1: seconds >= 30 carry the minute).
    pub const ADJ30: u8 = 0x08;
    /// Software-writable bits: HOLD and 30s-ADJ; BUSY and IRQ-FLAG persist
    /// (MAME: `m_reg[0] = (data & 0x09) | (m_reg[0] & 0x06)`).
    pub const WRITE_MASK: u8 = 0x09;
    pub const KEEP_MASK: u8 = 0x06;
}

/// CF (control F) bits.
mod cf {
    pub const RESET: u8 = 0x01;
    /// Stop the clock; releasing it resumes from the stopped value.
    pub const STOP: u8 = 0x02;
    /// 1 = 24-hour mode. Only changeable on a RESET 1 -> 0 transition (spec /
    /// MAME); power-on default is 24-hour (MAME `device_start`).
    pub const H24: u8 = 0x04;
    /// Writable bits besides the latched 24/12 bit.
    pub const WRITE_MASK: u8 = 0x0B;
}

/// Power-on control-register values (MAME `msm6242.cpp` `device_start`):
/// CD clear, CE = STD|t0, CF = 24-hour mode. `clock2_disto2.asm` never
/// initializes CF and relies on this default for 24-hour readout.
const CD_POWER_ON: u8 = 0x00;
const CE_POWER_ON: u8 = 0x06;
const CF_POWER_ON: u8 = cf::H24;

/// Hour of the AM->PM crossover, for the 12-hour conversions.
const NOON: u8 = 12;

/// Placeholder time the restored default `now` closure yields until the
/// frontend re-injects a real host time source through
/// [`DistoRTC::set_time_source`] — a sentinel epoch, not a guess at the real
/// time.
const RESTORED_PLACEHOLDER_TIME: RTCTime = RTCTime {
    year: 1970,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
};

/// `#[serde(default = "...")]` for [`MSM6242::now`]: yields [`RESTORED_PLACEHOLDER_TIME`]
/// until [`DistoRTC::set_time_source`] re-injects the real host clock.
fn default_time_source() -> TimeSource {
    Box::new(|| RESTORED_PLACEHOLDER_TIME)
}

/// An OKI MSM6242 real-time clock: 16 nibble-wide registers (BCD digit pairs
/// for second/minute/hour/day/month/year, a weekday counter, three control
/// registers). Reads follow MAME `msm6242.cpp` exactly; time-register writes
/// follow the real chip (MAME drops them — its time is not settable from the
/// guest, which would break NitrOS-9 `setime`).
///
/// The clock itself is `injected now() + offset_secs`: never ticked, so it
/// can't drift from the host clock and doesn't care about emulation pauses,
/// double-speed POKEs, or headless runs. Setting any time register moves the
/// offset.
#[derive(Serialize, Deserialize)]
pub struct MSM6242 {
    /// Never travels through a snapshot (a closure has no serializable
    /// shape) — skipped, restored to [`default_time_source`] until
    /// [`MSM6242::set_time_source`]/[`DistoRTC::set_time_source`]
    /// re-injects the real one.
    #[serde(skip, default = "default_time_source")]
    now: TimeSource,
    /// Emulated-clock minus host-clock, in seconds.
    offset_secs: i64,
    /// Snapshot served while CD HOLD is set (the running clock is unaffected).
    held: Option<i64>,
    /// Snapshot the clock is frozen at while CF STOP is set; time spent
    /// stopped is lost, like on the real chip.
    stopped: Option<i64>,
    reg_cd: u8,
    reg_ce: u8,
    reg_cf: u8,
}

impl std::fmt::Debug for MSM6242 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MSM6242")
            .field("offset_secs", &self.offset_secs)
            .field("held", &self.held)
            .field("stopped", &self.stopped)
            .field("cd", &self.reg_cd)
            .field("ce", &self.reg_ce)
            .field("cf", &self.reg_cf)
            .finish()
    }
}

impl MSM6242 {
    pub fn new(now: TimeSource) -> Self {
        Self {
            now,
            offset_secs: 0,
            held: None,
            stopped: None,
            reg_cd: CD_POWER_ON,
            reg_ce: CE_POWER_ON,
            reg_cf: CF_POWER_ON,
        }
    }

    /// The clock's current absolute time in epoch seconds (ignoring HOLD,
    /// which only freezes the register-file view).
    fn current_secs(&mut self) -> i64 {
        match self.stopped {
            Some(secs) => secs,
            None => (self.now)().to_secs() + self.offset_secs,
        }
    }

    /// The time the register file exposes: the HOLD snapshot if one is live.
    fn visible_secs(&mut self) -> i64 {
        match self.held {
            Some(secs) => secs,
            None => self.current_secs(),
        }
    }

    /// Move the clock to `secs`, keeping it pinned there if stopped.
    fn commit_secs(&mut self, secs: i64) {
        if self.stopped.is_some() {
            self.stopped = Some(secs);
        } else {
            self.offset_secs = secs - (self.now)().to_secs();
        }
        if self.held.is_some() {
            self.held = Some(secs);
        }
    }

    /// Set the clock outright (frontend "sync to host" / initial seeding).
    pub fn set_time(&mut self, time: RTCTime) {
        self.commit_secs(time.to_secs());
    }

    /// The clock's current time (frontend display).
    pub fn time(&mut self) -> RTCTime {
        RTCTime::from_secs(self.current_secs())
    }

    /// Restore-path-only: re-inject the host time source after a snapshot restore
    /// (`now` is `#[serde(skip)]`). Resumes exactly where the snapshot left
    /// off, not at a fresh offset.
    pub fn set_time_source(&mut self, now: TimeSource) {
        self.now = now;
    }

    /// Read register `reg` (0-15). Returns a nibble; the upper data bits are 0
    /// (MAME: the 4-bit chip's bus returns the value zero-extended).
    pub fn read(&mut self, reg: u8) -> u8 {
        let t = RTCTime::from_secs(self.visible_secs());
        match reg {
            REG_S1 => t.second % 10,
            REG_S10 => t.second / 10,
            REG_MI1 => t.minute % 10,
            REG_MI10 => t.minute / 10,
            REG_H1 | REG_H10 => {
                let (hour, pm) = self.display_hour(t.hour);
                if reg == REG_H1 {
                    hour % 10
                } else {
                    (hour / 10) | (u8::from(pm) << 2)
                }
            }
            REG_D1 => t.day % 10,
            REG_D10 => t.day / 10,
            REG_MO1 => t.month % 10,
            REG_MO10 => t.month / 10,
            REG_Y1 => (t.year.rem_euclid(100) % 10) as u8,
            REG_Y10 => (t.year.rem_euclid(100) / 10) as u8,
            REG_W => t.weekday(),
            REG_CD => self.reg_cd,
            REG_CE => self.reg_ce,
            REG_CF => self.reg_cf,
            _ => 0x00,
        }
    }

    /// The hour as the register file shows it: as-is in 24-hour mode; 12-hour
    /// mode folds to 1-12 with a PM flag (MAME `msm6242.cpp` read, H1/H10).
    fn display_hour(&self, hour: u8) -> (u8, bool) {
        if self.reg_cf & cf::H24 != 0 {
            return (hour, false);
        }
        let pm = hour >= NOON;
        let folded = hour % NOON;
        (if folded == 0 { NOON } else { folded }, pm)
    }

    /// Write `val` to register `reg` (0-15; only the low nibble matters).
    pub fn write(&mut self, reg: u8, val: u8) {
        let val = val & 0x0F;
        match reg {
            REG_CD => {
                if val & cd::ADJ30 != 0 {
                    self.adjust_30s();
                }
                let was_held = self.reg_cd & cd::HOLD != 0;
                let hold = val & cd::HOLD != 0;
                if hold && !was_held {
                    self.held = Some(self.current_secs());
                } else if !hold {
                    self.held = None;
                }
                self.reg_cd = (val & cd::WRITE_MASK) | (self.reg_cd & cd::KEEP_MASK);
            }
            REG_CE => self.reg_ce = val,
            REG_CF => {
                // 12/24 latches only on RESET 1->0 (spec); unlike MAME's
                // bugged transcription, we store what was written.
                if val & cf::RESET == 0 && self.reg_cf & cf::RESET != 0 {
                    self.reg_cf = val & (cf::WRITE_MASK | cf::H24);
                } else {
                    self.reg_cf = (val & cf::WRITE_MASK) | (self.reg_cf & cf::H24);
                }
                let stopping = val & cf::STOP != 0;
                if stopping {
                    if self.stopped.is_none() {
                        self.stopped = Some(self.current_secs());
                    }
                } else if let Some(secs) = self.stopped.take() {
                    self.offset_secs = secs - (self.now)().to_secs();
                }
            }
            REG_W => {} // weekday is derived from the date, not stored
            _ => self.write_time_digit(reg, val),
        }
    }

    /// Round to the nearest minute (CD 30s-ADJ): seconds >= 30 carry into the
    /// minute, then zero.
    fn adjust_30s(&mut self) {
        let secs = self.visible_secs();
        let rounded = if secs.rem_euclid(SECS_PER_MIN) >= 30 {
            secs + SECS_PER_MIN - secs.rem_euclid(SECS_PER_MIN)
        } else {
            secs - secs.rem_euclid(SECS_PER_MIN)
        };
        self.commit_secs(rounded);
    }

    /// Replace one BCD digit of the running time (real-chip behavior; NitrOS-9
    /// `setime` writes the file digit by digit, usually under HOLD).
    fn write_time_digit(&mut self, reg: u8, val: u8) {
        let mut t = RTCTime::from_secs(self.visible_secs());
        match reg {
            REG_S1 => t.second = t.second / 10 * 10 + val,
            REG_S10 => t.second = val * 10 + t.second % 10,
            REG_MI1 => t.minute = t.minute / 10 * 10 + val,
            REG_MI10 => t.minute = val * 10 + t.minute % 10,
            REG_H1 | REG_H10 => {
                let (mut hour, mut pm) = self.display_hour(t.hour);
                if reg == REG_H1 {
                    hour = hour / 10 * 10 + val;
                } else {
                    pm = val & 0x4 != 0;
                    hour = (val & 0x3) * 10 + hour % 10;
                }
                t.hour = self.stored_hour(hour, pm);
            }
            REG_D1 => t.day = t.day / 10 * 10 + val,
            REG_D10 => t.day = val * 10 + t.day % 10,
            REG_MO1 => t.month = t.month / 10 * 10 + val,
            REG_MO10 => t.month = val * 10 + t.month % 10,
            REG_Y1 => t.year = t.year - (t.year.rem_euclid(100) % 10) + i32::from(val),
            REG_Y10 => {
                let yy = t.year.rem_euclid(100);
                t.year = t.year - yy + i32::from(val) * 10 + yy % 10;
            }
            _ => return,
        }
        self.commit_secs(t.to_secs());
    }

    /// Inverse of [`MSM6242::display_hour`]: what to store for an hour written
    /// in the current mode.
    fn stored_hour(&self, hour: u8, pm: bool) -> u8 {
        if self.reg_cf & cf::H24 != 0 {
            return hour.min(23);
        }
        match (hour % NOON, pm) {
            (0, false) => 0,       // 12 AM -> 00
            (0, true) => NOON,     // 12 PM -> 12
            (h, false) => h,       // 1-11 AM
            (h, true) => h + NOON, // 1-11 PM -> 13-23
        }
    }
}

// ---- Disto MEB decode -------------------------------------------------------

const RTC_DATA: u16 = 0xFF50;
const RTC_SELECT: u16 = 0xFF51;
/// `$FF52`/`$FF53` are also address-latch writes (MAME `meb_rtime.cpp` — the
/// 2-N-1's NitrOS-9 driver selects through `$FF52`); on the real card they
/// double as the Centronics strobe/BUSY port, not modeled here.
const RTC_SELECT_ALT: u16 = 0xFF52;
const RTC_SELECT_ALT2: u16 = 0xFF53;

/// The Disto MEB real-time clock as a cartridge-port device: an [`MSM6242`]
/// behind a one-byte address latch at `$FF50-$FF53` in the SCS window. Rides
/// the existing cartridge routing — plug it into the port directly (NitrOS-9
/// boots from VHD without a disk controller) or into a Multi-Pak slot next to
/// the FD-502, as the real MEB host cards did.
#[derive(Serialize, Deserialize)]
pub struct DistoRTC {
    rtc: MSM6242,
    address_latch: u8,
}

impl std::fmt::Debug for DistoRTC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DistoRTC")
            .field("address_latch", &self.address_latch)
            .field("rtc", &self.rtc)
            .finish()
    }
}

impl DistoRTC {
    /// Build the cart around a host time source; starts at the source's time
    /// (offset 0), like a battery-backed chip that was already set.
    pub fn new(now: TimeSource) -> Self {
        Self {
            rtc: MSM6242::new(now),
            address_latch: 0,
        }
    }

    /// Direct access to the clock chip (frontend set/sync UI).
    pub fn rtc(&mut self) -> &mut MSM6242 {
        &mut self.rtc
    }

    /// Restore-path-only: re-inject the host time source after a snapshot restore.
    pub fn set_time_source(&mut self, now: TimeSource) {
        self.rtc.set_time_source(now);
    }
}

impl Cartridge for DistoRTC {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            RTC_DATA => self.rtc.read(self.address_latch & 0x0F),
            // Centronics BUSY in bit 7; no printer attached, never busy.
            RTC_SELECT_ALT | RTC_SELECT_ALT2 => 0x00,
            _ => IO_OPEN_BUS,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            RTC_DATA => self.rtc.write(self.address_latch & 0x0F, val),
            RTC_SELECT | RTC_SELECT_ALT | RTC_SELECT_ALT2 => self.address_latch = val,
            _ => {}
        }
    }

    // No `reset` override: the chip is battery-backed, RESET* doesn't touch the registers.
}

#[cfg(test)]
#[path = "rtc_test.rs"]
mod tests;
