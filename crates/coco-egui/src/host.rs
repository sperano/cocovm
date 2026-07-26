use crate::*;

/// The host's local wall clock, read once (RTC sync).
pub(crate) fn host_now() -> RTCTime {
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

/// The DriveWire server's injected wall clock (`drivewire::DwClock`,
/// `coco_core::drivewire` — like [`host_time_source`], coco-core itself
/// never reads `std::time`). Shared by [`CocoApp::enable_drivewire`] and
/// `CocoApp::load_state_from`'s restore path (`save_state.rs`) — a restored
/// `DwServer`'s clock is `#[serde(skip)]`, same reasoning as the Disto RTC's
/// time source.
pub(crate) fn host_dw_clock() -> drivewire::DwClock {
    Box::new(|| {
        let now = chrono::Local::now();
        DwTime {
            year: now.year() as u16,
            month: now.month() as u8,
            day: now.day() as u8,
            hour: now.hour() as u8,
            minute: now.minute() as u8,
            second: now.second() as u8,
        }
    })
}
