//! Disto 4-N-1 real-time clock: an OKI MSM6242 (`msm6242.rs`) decoded in the
//! SCS window at `$FF50-$FF53` (MAME `meb_rtime.cpp` `disto_rtime_device`).
//! NitrOS-9 `clock2_disto4.asm` selects through `$FF51`; verified live
//! against that driver (`date`/`setime`).
//!
//! Register interface:
//! - `$FF50` read/write — data register, indexed by the address latch.
//! - `$FF51`, `$FF52`, `$FF53` write — address latch. On the real cards
//!   `$FF52`/`$FF53` double as the Centronics printer strobe, not modeled;
//!   reads return the printer BUSY line (never busy) in bit 7.
//!
//! Time never comes from `std::time` (coco-core stays host-clock-free): the
//! frontend injects a [`TimeSource`] closure, and the chip tracks it through a
//! signed offset so NitrOS-9's `setime` (writes to the time registers, real
//! chip behavior; MAME drops these writes) sticks without the emulated
//! clock drifting when the machine is paused or the CPU runs double-speed.

pub mod msm6242;

use serde::{Deserialize, Serialize};

use crate::cart::{Cartridge, IO_OPEN_BUS};
pub use msm6242::MSM6242;

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

// ---- Disto MEB decode -------------------------------------------------------

const RTC_DATA: u16 = 0xFF50;
const RTC_SELECT: u16 = 0xFF51;
/// `$FF52`/`$FF53` are also address-latch writes (MAME `meb_rtime.cpp`); on
/// the real card they double as the Centronics strobe/BUSY port, not modeled.
const RTC_SELECT_ALT: u16 = 0xFF52;
const RTC_SELECT_ALT2: u16 = 0xFF53;

/// The Disto 4-N-1 real-time clock as a cartridge-port device: a clock chip
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
    /// A clock around a host time source; starts at the source's time
    /// (offset 0), like a battery-backed chip that was already set.
    pub fn new(now: TimeSource) -> Self {
        Self {
            rtc: MSM6242::new(now),
            address_latch: 0,
        }
    }

    /// Direct access to the clock core (frontend set/sync UI).
    pub fn rtc(&mut self) -> &mut MSM6242 {
        &mut self.rtc
    }

    /// Restore-path-only: re-inject the host time source after a snapshot restore.
    pub fn set_time_source(&mut self, now: TimeSource) {
        self.rtc.set_time_source(now);
    }

    fn read_reg(&mut self) -> u8 {
        self.rtc.read(self.address_latch & 0x0F)
    }

    fn write_reg(&mut self, val: u8) {
        self.rtc.write(self.address_latch & 0x0F, val);
    }
}

impl Cartridge for DistoRTC {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            RTC_DATA => self.read_reg(),
            // Centronics BUSY in bit 7; no printer attached, never busy.
            RTC_SELECT_ALT | RTC_SELECT_ALT2 => 0x00,
            _ => IO_OPEN_BUS,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            RTC_DATA => self.write_reg(val),
            RTC_SELECT | RTC_SELECT_ALT | RTC_SELECT_ALT2 => self.address_latch = val,
            _ => {}
        }
    }

    // No `reset` override: the chip is battery-backed, RESET* doesn't touch the registers.
}

#[cfg(test)]
#[path = "rtc_test.rs"]
mod tests;
