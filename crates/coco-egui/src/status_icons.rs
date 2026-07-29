//! Status-bar device-activity icons: small silhouettes painted before each
//! device's label ([`crate::chrome::status_bar`]), red while the device is
//! active and gray while idle, plus the UI-side latching that turns a
//! monotonic activity counter (bytes transferred, sectors read, …) into a
//! visible pulse.
//!
//! Every icon here follows the same recipe: a filled silhouette in
//! [`ICON_ACTIVE`]/[`ICON_IDLE`], with fine detail "punched" out of it in
//! `ui.visuals().panel_fill` (the status bar's own background) rather than
//! drawn as separate strokes — cheap to paint (one filled path per shape)
//! and correct against both light and dark themes for free.

use coco_core::drivewire;
use eframe::egui;

use crate::widgets::UI_DRIVES;

/// Icon silhouette color while its device is active — the same red used for
/// every activity light in the status bar.
pub(crate) const ICON_ACTIVE: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x30, 0x30);

/// Icon silhouette color while its device is idle.
pub(crate) const ICON_IDLE: egui::Color32 = egui::Color32::from_gray(70);

/// How long an [`ActivityLatch`] keeps reporting "active" after the last
/// counter change it observed — long enough that a single sector
/// read/write or byte transfer reads as a visible pulse rather than a
/// single-frame flicker.
// `ActivityLatch::observe` (the only non-test call site that exercises this
// transitively) has no caller yet — its first `*_status` call site lands in
// the VHD-activity commit — so this reads as dead code to rustc until then.
#[allow(dead_code)]
pub(crate) const ACTIVITY_HOLD: std::time::Duration = std::time::Duration::from_millis(200);

/// Status-bar floppy activity indicator: a little 5¼" floppy jacket, red
/// while the drive is selected with its motor on ([`coco_core::fdc`]'s
/// `drive_active`, like a real drive's front-panel light), gray otherwise.
pub(crate) const DRIVE_ICON_SIZE: f32 = 14.0;

/// Corner rounding of the jacket square.
pub(crate) const DRIVE_ICON_CORNER: f32 = 1.5;

/// Status-bar cassette activity indicator, the tape sibling of
/// [`DRIVE_ICON_SIZE`]'s floppy: shell proportions of a compact cassette
/// (wider than tall), red while the cassette relay is closed
/// (CLOAD/CSAVE/`MOTOR ON`), gray otherwise.
pub(crate) const TAPE_ICON_SIZE: egui::Vec2 = egui::vec2(18.0, 13.0);

/// Corner rounding of the cassette shell.
pub(crate) const TAPE_ICON_CORNER: f32 = 1.5;

/// Radius of each reel's punched hub circle, as drawn by [`cassette_icon`].
const REEL_HUB_R: f32 = TAPE_ICON_SIZE.y * 0.20;

/// Reel rotation credited per tape byte of playback movement (CLOAD/CSAVE
/// read position advancing or rewinding — [`crate::CocoApp::tape_status`]'s
/// `cassette.position()`). Not a physically accurate angular rate (a real
/// reel's angular speed changes with how much tape is spooled on it) — this
/// is a status-icon approximation picked so a reel visibly turns at typical
/// CLOAD speeds (one full turn per 40 bytes) without spinning distractingly
/// fast.
const REEL_ANGLE_PER_BYTE: f32 = std::f32::consts::TAU / 40.0;

/// Reel rotation speed, in radians/second, while the motor runs but the
/// tape position isn't moving: CSAVE (recording never advances
/// `Cassette::position`'s `pos`) and the brief motor spin-up before
/// playback position starts moving. Picked to look like a cassette motor at
/// speed, not calibrated against a real deck.
const RECORD_REEL_SPEED: f32 = std::f32::consts::TAU * 0.8;

/// Number of spokes drawn on each reel hub.
const REEL_SPOKE_COUNT: u32 = 3;

/// Spoke length, as a fraction of [`REEL_HUB_R`].
const REEL_SPOKE_LEN_FRAC: f32 = 0.85;

/// Advance the cassette status icon's reel angle by one frame. Pure
/// (doesn't touch `ui` or `self`) so it can be unit-tested directly; called
/// from `tape_status` with the app's stored angle/position and the frame's
/// `dt`, and the returned angle/position are stored back.
///
/// - If tape position moved since last frame (`pos != last_pos`), the reels
///   turn by the moved distance (`REEL_ANGLE_PER_BYTE` per byte) — forward
///   for playback, backward for a rewind. `pos` and `last_pos` are byte
///   offsets (never negative), so the signed distance moved is recovered by
///   reinterpreting a wrapping subtraction as `isize` rather than by
///   subtracting directly, which would panic/wrap on a rewind.
/// - Else, if the motor is running (position parked but the relay is
///   closed — CSAVE, or spin-up before playback moves), the reels keep
///   turning at [`RECORD_REEL_SPEED`].
/// - Else (motor off), the angle is unchanged: the reels park.
///
/// The returned angle is wrapped into `0..TAU`.
pub(crate) fn next_reel_angle(
    angle: f32,
    last_pos: usize,
    pos: usize,
    motor: bool,
    dt: f32,
) -> (f32, usize) {
    let angle = if pos != last_pos {
        let delta = pos.wrapping_sub(last_pos) as isize;
        angle + delta as f32 * REEL_ANGLE_PER_BYTE
    } else if motor {
        angle + dt * RECORD_REEL_SPEED
    } else {
        angle
    };
    (angle.rem_euclid(std::f32::consts::TAU), pos)
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
    /// Observe the current value of the counter this latch is tracking.
    /// Returns whether the icon should currently draw active (see the
    /// type's doc comment for the priming/hold/decrease rules).
    // No `*_status` fn calls this yet — the VHD-activity commit adds the
    // first one — so unlike `observe_at` (exercised directly by
    // status_icons_test.rs), rustc sees this as dead until then.
    #[allow(dead_code)]
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
        self.last_change.is_some_and(|t| now.duration_since(t) < ACTIVITY_HOLD)
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
    // Each latch below is read starting from the commit that wires up its
    // device's `*_status` fn; until then it's written by `Default` alone
    // and rustc flags it dead. Attributes come off one at a time as each
    // commit lands.
    #[allow(dead_code)]
    pub(crate) vhd: [ActivityLatch; UI_DRIVES],
    #[allow(dead_code)]
    pub(crate) dw: [ActivityLatch; drivewire::DRIVE_COUNT],
    #[allow(dead_code)]
    pub(crate) rs232_tx: ActivityLatch,
    #[allow(dead_code)]
    pub(crate) rs232_rx: ActivityLatch,
    #[allow(dead_code)]
    pub(crate) printer: ActivityLatch,
    /// Current cassette reel rotation, in radians (see [`next_reel_angle`]).
    pub(crate) tape_reel_angle: f32,
    /// Tape playback position (in tape bytes) as of the last frame's
    /// [`next_reel_angle`] call, so the next frame can tell how far it moved.
    pub(crate) tape_last_pos: usize,
}

/// One status-bar cassette indicator (see [`TAPE_ICON_SIZE`]'s doc): the
/// shell with the two reel hubs punched out in the panel's background
/// color, each with [`REEL_SPOKE_COUNT`] spokes drawn back in the shell
/// color at `reel_angle` — both reels always at the same angle, since real
/// cassette reels are pulled by the same capstan/pinch-roller and co-rotate
/// (linked by the tape between them, not independent motors).
pub(crate) fn cassette_icon(ui: &mut egui::Ui, active: bool, reel_angle: f32) {
    let (rect, _) = ui.allocate_exact_size(TAPE_ICON_SIZE, egui::Sense::hover());
    let shell = if active { ICON_ACTIVE } else { ICON_IDLE };
    let punch = ui.visuals().panel_fill;
    let painter = ui.painter();
    painter.rect_filled(rect, TAPE_ICON_CORNER, shell);
    // The two reel hubs, side by side above the mid-line (the head window
    // occupies a real shell's bottom edge, unreadable at this size).
    let hub_y = rect.center().y - TAPE_ICON_SIZE.y * 0.08;
    let hub_dx = TAPE_ICON_SIZE.x * 0.22;
    for hub_x in [rect.center().x - hub_dx, rect.center().x + hub_dx] {
        let hub = egui::pos2(hub_x, hub_y);
        painter.circle_filled(hub, REEL_HUB_R, punch);
        draw_reel_spokes(painter, hub, reel_angle, shell);
    }
}

/// The spokes punched back into a reel hub in `color` (the shell color),
/// evenly spaced around `reel_angle` — see [`next_reel_angle`] for how the
/// angle advances frame to frame.
fn draw_reel_spokes(painter: &egui::Painter, hub: egui::Pos2, reel_angle: f32, color: egui::Color32) {
    let len = REEL_HUB_R * REEL_SPOKE_LEN_FRAC;
    let stroke = egui::Stroke::new(1.0f32, color);
    for k in 0..REEL_SPOKE_COUNT {
        let theta = reel_angle + k as f32 * std::f32::consts::TAU / REEL_SPOKE_COUNT as f32;
        let tip = hub + len * egui::vec2(theta.cos(), theta.sin());
        painter.line_segment([hub, tip], stroke);
    }
}

/// One status-bar floppy activity indicator (see [`DRIVE_ICON_SIZE`]'s
/// doc): the jacket square with the hub hole, the oblong head-access slot,
/// the index-hole dot, and the write-protect notch all punched out in the
/// panel's background color — the 5¼" silhouette.
pub(crate) fn floppy_icon(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(DRIVE_ICON_SIZE, DRIVE_ICON_SIZE),
        egui::Sense::hover(),
    );
    let jacket = if active { ICON_ACTIVE } else { ICON_IDLE };
    let punch = ui.visuals().panel_fill;
    let painter = ui.painter();
    painter.rect_filled(rect, DRIVE_ICON_CORNER, jacket);
    // Hub hole, a hair above center (the slot below claims the bottom).
    let hub = rect.center() - egui::vec2(0.0, DRIVE_ICON_SIZE * 0.08);
    painter.circle_filled(hub, DRIVE_ICON_SIZE * 0.18, punch);
    // Head-access slot: the short oblong under the hub.
    let slot_width = DRIVE_ICON_SIZE * 0.16;
    let slot = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - DRIVE_ICON_SIZE * 0.18),
        egui::vec2(slot_width, DRIVE_ICON_SIZE * 0.24),
    );
    painter.rect_filled(slot, slot_width / 2.0, punch);
    // Index hole: a small dot out on the hub's radius, at the angle a real
    // 5¼" jacket's index-sensor window sits at (drive-side, upper right).
    let index = hub + egui::vec2(DRIVE_ICON_SIZE * 0.22, 0.0);
    painter.circle_filled(index, DRIVE_ICON_SIZE * 0.05, punch);
    // Write-protect notch: a small rectangular nick in the jacket's right
    // edge (covering it on a real 5¼" disk write-protects the drive).
    let notch = egui::Rect::from_min_size(
        egui::pos2(rect.right() - DRIVE_ICON_SIZE * 0.12, rect.center().y - DRIVE_ICON_SIZE * 0.09),
        egui::vec2(DRIVE_ICON_SIZE * 0.12, DRIVE_ICON_SIZE * 0.18),
    );
    painter.rect_filled(notch, 0.0, punch);
}

#[cfg(test)]
#[path = "status_icons_test.rs"]
mod tests;
