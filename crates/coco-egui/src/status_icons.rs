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
    /// Current cassette reel rotation, in radians (see `next_reel_angle`,
    /// added alongside the spinning-reel detail).
    #[allow(dead_code)]
    pub(crate) tape_reel_angle: f32,
    /// Tape playback position (in tape bytes) as of the last frame's
    /// `next_reel_angle` call, so the next frame can tell how far it moved.
    #[allow(dead_code)]
    pub(crate) tape_last_pos: usize,
}

/// One status-bar cassette indicator (see [`TAPE_ICON_SIZE`]'s doc): the
/// shell with the two reel hubs punched out in the panel's background
/// color. `reel_angle` is accepted for the reel-spin detail added on top of
/// this shell later; unused for now.
pub(crate) fn cassette_icon(ui: &mut egui::Ui, active: bool, _reel_angle: f32) {
    let (rect, _) = ui.allocate_exact_size(TAPE_ICON_SIZE, egui::Sense::hover());
    let shell = if active { ICON_ACTIVE } else { ICON_IDLE };
    let punch = ui.visuals().panel_fill;
    let painter = ui.painter();
    painter.rect_filled(rect, TAPE_ICON_CORNER, shell);
    // The two reel hubs, side by side above the mid-line (the head window
    // occupies a real shell's bottom edge, unreadable at this size).
    let hub_y = rect.center().y - TAPE_ICON_SIZE.y * 0.08;
    let hub_dx = TAPE_ICON_SIZE.x * 0.22;
    let hub_r = TAPE_ICON_SIZE.y * 0.20;
    painter.circle_filled(egui::pos2(rect.center().x - hub_dx, hub_y), hub_r, punch);
    painter.circle_filled(egui::pos2(rect.center().x + hub_dx, hub_y), hub_r, punch);
}

/// One status-bar floppy activity indicator (see [`DRIVE_ICON_SIZE`]'s
/// doc): the jacket square with the hub hole and the oblong head-access
/// slot punched out in the panel's background color — the 5¼" silhouette.
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
}

#[cfg(test)]
#[path = "status_icons_test.rs"]
mod tests;
