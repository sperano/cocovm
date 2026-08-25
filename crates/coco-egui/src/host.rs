use crate::*;

/// The host's local wall clock — [`host_time_source`]'s closure re-reads it
/// on every RTC register access.
fn host_now() -> RTCTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    RTCTime {
        year: now.year(),
        month: now.month() as u8,
        day: now.day() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
    }
}

/// [`host_now`] as the Disto RTC's injected time source
/// (`coco_core::rtc::TimeSource` — coco-core itself never reads `std::time`).
pub(crate) fn host_time_source() -> coco_core::rtc::TimeSource {
    Box::new(host_now)
}

/// The DriveWire server's injected wall clock — like [`host_time_source`],
/// coco-core never reads `std::time` directly. A restored `DwServer`'s clock
/// is `#[serde(skip)]` for the same reason.
pub(crate) fn host_dw_clock() -> drivewire::DWClock {
    Box::new(|| {
        let now = chrono::Local::now();
        DWTime {
            year: now.year() as u16,
            month: now.month() as u8,
            day: now.day() as u8,
            hour: now.hour() as u8,
            minute: now.minute() as u8,
            second: now.second() as u8,
        }
    })
}
