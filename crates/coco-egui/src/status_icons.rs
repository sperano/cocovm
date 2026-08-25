//! Status-bar device-activity icons: small silhouettes painted before each
//! device's label ([`crate::CocoApp::status_bar_ui`]), red while the device is
//! active and gray while idle, plus the UI-side latching that turns a
//! monotonic activity counter (bytes transferred, sectors read, …) into a
//! visible pulse.
//!
//! This module owns the latching/state side ([`ActivityLatch`],
//! [`StatusActivity`], the cassette reel's [`TapeReel`]); the silhouette
//! painting itself — and every icon's geometry — lives in [`paint`].

mod paint;

pub(crate) use paint::{
    cart_icon, cassette_icon, drivewire_icon, floppy_icon, joystick_icon, keyboard_icon,
    monitor_icon, mpi_icon, printer_icon, rs232_icon, tv_icon, vhd_icon,
};

use coco_core::{drivewire, vhd};
use eframe::egui;

/// Icon silhouette color while its device is active — the same red used for
/// every activity light in the status bar.
const ICON_ACTIVE: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x30, 0x30);

/// Icon silhouette color while its device is idle.
const ICON_IDLE: egui::Color32 = egui::Color32::from_gray(70);

/// How long an [`ActivityLatch`] keeps reporting "active" after the last
/// counter change it observed — long enough that a single sector
/// read/write or byte transfer reads as a visible pulse rather than a
/// single-frame flicker.
const ACTIVITY_HOLD: std::time::Duration = std::time::Duration::from_millis(200);

/// Reel rotation credited per tape byte of position movement (CLOAD's read
/// head advancing, CSAVE's live record count, or a rewind —
/// [`crate::CocoApp::tape_status`]'s `cassette.position()`). Not a
/// physically accurate angular rate (a real reel's angular speed changes
/// with how much tape is spooled on it) — this is a status-icon
/// approximation picked so a reel visibly turns at typical CLOAD speeds
/// (one full turn per 40 bytes) without spinning distractingly fast.
const REEL_ANGLE_PER_BYTE: f32 = std::f32::consts::TAU / 40.0;

/// Reel rotation speed, in radians/second, while the motor runs but the
/// tape position isn't moving: the spin-up stretch before playback or
/// recording starts moving the position, and `MOTOR ON` with the tape
/// parked (at its end, or nothing being written). Picked to look like a
/// cassette motor at speed, not calibrated against a real deck.
const MOTOR_REEL_SPEED: f32 = std::f32::consts::TAU * 0.8;

/// Cassette status icon's reel state: the current draw angle, plus the tape
/// byte position as of the last [`Self::advance`] call, so the next call
/// can tell how far the tape has moved since then. Lives on
/// [`StatusActivity`] as `tape_reel`.
#[derive(Default)]
pub(crate) struct TapeReel {
    angle: f32,
    last_pos: usize,
}

impl TapeReel {
    /// Advance the reel by one frame and return the angle to draw it at:
    /// turns by the moved distance if `pos` changed, else at
    /// [`MOTOR_REEL_SPEED`] if the motor's running, else parks. Wrapped into `0..TAU`.
    pub(crate) fn advance(&mut self, pos: usize, motor: bool, dt: f32) -> f32 {
        let angle = if pos != self.last_pos {
            let delta = pos.wrapping_sub(self.last_pos) as isize;
            self.angle + delta as f32 * REEL_ANGLE_PER_BYTE
        } else if motor {
            self.angle + dt * MOTOR_REEL_SPEED
        } else {
            self.angle
        };
        self.angle = angle.rem_euclid(std::f32::consts::TAU);
        self.last_pos = pos;
        self.angle
    }
}

/// UI-side pulse stretcher over a monotonic activity counter (bytes sent,
/// sectors read, …): turns "the counter changed at some point" into a
/// boolean an icon can light for [`ACTIVITY_HOLD`] after the fact, since a
/// counter bump itself is instantaneous and would otherwise never be
/// visible on screen.
///
/// The very first [`Self::observe`] call only *primes* the latch — it never
/// reports active, no matter what value it sees. Without this, restoring a
/// save state whose counter already sat at some large value (e.g.
/// `tx_bytes == 40000`) would light every activity icon for one frame on
/// load, even though nothing actually happened. Every observation after the
/// first lights the latch on ANY change, including a decrease: a counter
/// that rewinds (e.g. after loading an older save state) still means
/// something changed, so it blips once, the same as a forward change would.
///
/// Holds are judged lazily, at draw time, against the wall clock — nothing
/// schedules a repaint when a hold expires. That means after a long pause
/// (window not redrawn for a while) the light reads idle again immediately
/// on the next draw, without a stale "still lit" frame first.
#[derive(Default)]
pub(crate) struct ActivityLatch {
    last: Option<u64>,
    last_change: Option<std::time::Instant>,
}

impl ActivityLatch {
    /// Observe the current value of the counter this latch is tracking,
    /// returning whether the icon should draw active (see the type doc for the rules).
    pub(crate) fn observe(&mut self, count: u64) -> bool {
        self.observe_at(count, std::time::Instant::now())
    }

    /// [`Self::observe`] with an injected clock reading, so tests can
    /// control time directly instead of sleeping.
    fn observe_at(&mut self, count: u64, now: std::time::Instant) -> bool {
        match self.last {
            None => {
                // First-ever observation: prime only, never light.
                self.last = Some(count);
            }
            Some(last) if last != count => {
                self.last = Some(count);
                self.last_change = Some(now);
            }
            _ => {}
        }
        self.last_change
            .is_some_and(|t| now.duration_since(t) < ACTIVITY_HOLD)
    }
}

/// Every device the status bar can show an activity light for, latched
/// UI-side over the core's monotonic per-device counters (see
/// [`ActivityLatch`]). Lives on [`crate::CocoApp`] as `activity`; purely UI
/// state, so `CocoApp` not being serialized means there's nothing to worry
/// about saving/restoring here — a fresh app (or one built from a restored
/// snapshot) just starts every latch primed on its first draw.
#[derive(Default)]
pub(crate) struct StatusActivity {
    pub(crate) vhd: [ActivityLatch; vhd::DRIVE_COUNT],
    pub(crate) dw: [ActivityLatch; drivewire::DRIVE_COUNT],
    pub(crate) rs232_tx: ActivityLatch,
    pub(crate) rs232_rx: ActivityLatch,
    pub(crate) printer: ActivityLatch,
    /// The cassette status icon's reel state (see [`TapeReel`]).
    pub(crate) tape_reel: TapeReel,
}

#[cfg(test)]
#[path = "status_icons_test.rs"]
mod tests;
