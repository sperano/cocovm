//! The status bar's device silhouettes: one painter function per device
//! (`crate::chrome::status_bar`'s call sites), plus the geometry each one
//! draws to. Every icon follows the same recipe: a filled silhouette in
//! [`super::ICON_ACTIVE`]/[`super::ICON_IDLE`], with fine detail "punched"
//! out of it in `ui.visuals().panel_fill` (the status bar's own
//! background) rather than drawn as separate strokes — cheap to paint (one
//! filled path per shape) and correct against both light and dark themes
//! for free.
//!
//! Named consts cover every size, speed, and threshold, per the project's
//! usual convention — but each icon's *internal* proportions (a hub sitting
//! at `0.22 * size`, a notch at `0.72 * size`, …) are left as inline
//! numeric fractions of that icon's own size rather than promoted to
//! consts: they have no meaning outside the shape they scale and naming
//! them would just multiply the const count without adding information a
//! reader doesn't already get from seeing the fraction next to the size it
//! multiplies.

use eframe::egui;

use super::{ICON_ACTIVE, ICON_IDLE};

/// Shared setup for every status-bar device icon: allocates the icon's
/// rect, resolves the shell color (red/gray per `active`) and the panel
/// background used to "punch" detail out of the shell, and hands back the
/// `egui::Painter` to draw with, plus the allocation's `Response` — every
/// painter returns this so its call site in `crate::chrome::status_bar` can
/// attach a hover tooltip — the four-line preamble every painter below
/// would otherwise repeat.
struct Icon<'a> {
    rect: egui::Rect,
    shell: egui::Color32,
    punch: egui::Color32,
    painter: &'a egui::Painter,
    response: egui::Response,
}

fn begin_icon(ui: &mut egui::Ui, size: egui::Vec2, active: bool) -> Icon<'_> {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    Icon {
        rect,
        shell: if active { ICON_ACTIVE } else { ICON_IDLE },
        punch: ui.visuals().panel_fill,
        painter: ui.painter(),
        response,
    }
}

/// Status-bar cassette icon size: shell proportions of a compact cassette,
/// wider than tall.
const TAPE_ICON_SIZE: egui::Vec2 = egui::vec2(18.0, 13.0);

/// Corner rounding of the cassette shell.
const TAPE_ICON_CORNER: f32 = 1.5;

/// Radius of each reel's punched hub circle, as drawn by [`cassette_icon`].
const REEL_HUB_R: f32 = TAPE_ICON_SIZE.y * 0.20;

/// Number of spokes drawn on each reel hub.
const REEL_SPOKE_COUNT: u32 = 3;

/// Spoke length, as a fraction of [`REEL_HUB_R`].
const REEL_SPOKE_LEN_FRAC: f32 = 0.85;

/// Status-bar cassette activity indicator (see [`TAPE_ICON_SIZE`]): red
/// while the cassette relay is closed (CLOAD/CSAVE/`MOTOR ON`), gray
/// otherwise. The shell has the two reel hubs punched out in the panel's
/// background color, each with [`REEL_SPOKE_COUNT`] spokes drawn back in
/// the shell color at `reel_angle` — both reels always at the same angle,
/// since real cassette reels are pulled by the same capstan/pinch-roller
/// and co-rotate (linked by the tape between them, not independent
/// motors).
pub(crate) fn cassette_icon(ui: &mut egui::Ui, active: bool, reel_angle: f32) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, TAPE_ICON_SIZE, active);
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
    response
}

/// The spokes punched back into a reel hub in `color` (the shell color),
/// evenly spaced around `reel_angle` — see [`super::TapeReel::advance`] for
/// how the angle advances frame to frame.
fn draw_reel_spokes(painter: &egui::Painter, hub: egui::Pos2, reel_angle: f32, color: egui::Color32) {
    let len = REEL_HUB_R * REEL_SPOKE_LEN_FRAC;
    let stroke = egui::Stroke::new(1.0f32, color);
    for k in 0..REEL_SPOKE_COUNT {
        let theta = reel_angle + k as f32 * std::f32::consts::TAU / REEL_SPOKE_COUNT as f32;
        let tip = hub + len * egui::vec2(theta.cos(), theta.sin());
        painter.line_segment([hub, tip], stroke);
    }
}

/// Status-bar floppy icon size: a little 5¼" floppy jacket.
const DRIVE_ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 14.0);

/// Corner rounding of the jacket square.
const DRIVE_ICON_CORNER: f32 = 1.5;

/// Status-bar floppy activity indicator (see [`DRIVE_ICON_SIZE`]): red
/// while the drive is selected with its motor on ([`coco_core::fdc`]'s
/// `drive_active`, like a real drive's front-panel light), gray otherwise.
/// The jacket square has the hub hole, the oblong head-access slot, the
/// index-hole dot, and the write-protect notch all punched out in the
/// panel's background color — the 5¼" silhouette.
pub(crate) fn floppy_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, DRIVE_ICON_SIZE, active);
    painter.rect_filled(rect, DRIVE_ICON_CORNER, shell);
    // Hub hole, a hair above center (the slot below claims the bottom).
    let hub = rect.center() - egui::vec2(0.0, DRIVE_ICON_SIZE.x * 0.08);
    painter.circle_filled(hub, DRIVE_ICON_SIZE.x * 0.18, punch);
    // Head-access slot: the short oblong under the hub.
    let slot_width = DRIVE_ICON_SIZE.x * 0.16;
    let slot = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - DRIVE_ICON_SIZE.x * 0.18),
        egui::vec2(slot_width, DRIVE_ICON_SIZE.x * 0.24),
    );
    painter.rect_filled(slot, slot_width / 2.0, punch);
    // Index hole: a small dot out on the hub's radius, at the angle a real
    // 5¼" jacket's index-sensor window sits at (drive-side, upper right).
    let index = hub + egui::vec2(DRIVE_ICON_SIZE.x * 0.22, 0.0);
    painter.circle_filled(index, DRIVE_ICON_SIZE.x * 0.05, punch);
    // Write-protect notch: a small rectangular nick in the jacket's right
    // edge (covering it on a real 5¼" disk write-protects the drive).
    let notch = egui::Rect::from_min_size(
        egui::pos2(
            rect.right() - DRIVE_ICON_SIZE.x * 0.12,
            rect.center().y - DRIVE_ICON_SIZE.x * 0.09,
        ),
        egui::vec2(DRIVE_ICON_SIZE.x * 0.12, DRIVE_ICON_SIZE.x * 0.18),
    );
    painter.rect_filled(notch, 0.0, punch);
    response
}

/// Status-bar VHD icon size: a 3½" hard-drive top-view silhouette.
const VHD_ICON_SIZE: egui::Vec2 = egui::vec2(15.0, 11.0);

/// Corner rounding of the VHD drive housing.
const VHD_ICON_CORNER: f32 = 1.5;

/// Status-bar VHD (virtual hard disk, `$FF80-$FF86` `emudsk`) activity
/// indicator (see [`VHD_ICON_SIZE`]): red while a READ/WRITE/FLUSH command
/// has recently dispatched to that drive
/// ([`coco_core::vhd::VHD::access_count`]), gray otherwise. The housing
/// rectangle has one large platter circle — offset toward the left edge,
/// the way a real 3½" drive's platter sits off-center under its own
/// top-view case — punched out in the panel background color, a
/// shell-colored hub dot at the platter's center, and a thin shell-colored
/// actuator-arm line reaching from the housing's bottom-right corner onto
/// the platter, the way a hard drive's read/write head arm does.
pub(crate) fn vhd_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, VHD_ICON_SIZE, active);
    painter.rect_filled(rect, VHD_ICON_CORNER, shell);

    let platter_r = VHD_ICON_SIZE.y * 0.42;
    let platter_center = egui::pos2(rect.left() + VHD_ICON_SIZE.y * 0.55, rect.center().y);
    painter.circle_filled(platter_center, platter_r, punch);
    painter.circle_filled(platter_center, VHD_ICON_SIZE.y * 0.08, shell);

    let arm_stroke = egui::Stroke::new(1.0f32, shell);
    painter.line_segment([rect.right_bottom(), platter_center], arm_stroke);
    response
}

/// Status-bar DriveWire icon size: a serial-cable-and-plug silhouette.
const DW_ICON_SIZE: egui::Vec2 = egui::vec2(15.0, 10.0);

/// Status-bar DriveWire activity indicator (see [`DW_ICON_SIZE`]): red
/// while a sector has recently been read from or written to that drive
/// over the Becker port ([`coco_core::drivewire::DWServer::drive_ops`]),
/// gray otherwise. A small plug body sits at the right with two punched
/// pin slots, and a shell-colored cable line runs from the plug to the
/// icon's left edge with one sag/kink partway along, evoking a serial
/// cable running off to the host.
pub(crate) fn drivewire_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, DW_ICON_SIZE, active);

    // Plug body: a small rect hugging the right edge.
    let plug_w = DW_ICON_SIZE.x * 0.4;
    let plug = egui::Rect::from_min_size(
        egui::pos2(rect.right() - plug_w, rect.center().y - DW_ICON_SIZE.y * 0.35),
        egui::vec2(plug_w, DW_ICON_SIZE.y * 0.7),
    );
    painter.rect_filled(plug, 1.0, shell);
    // Two pin slots punched into the plug face.
    let pin_w = plug_w * 0.22;
    let pin_h = DW_ICON_SIZE.y * 0.32;
    for dy in [-DW_ICON_SIZE.y * 0.16, DW_ICON_SIZE.y * 0.16] {
        let pin = egui::Rect::from_center_size(
            egui::pos2(plug.center().x, rect.center().y + dy),
            egui::vec2(pin_w, pin_h),
        );
        painter.rect_filled(pin, 0.0, punch);
    }

    // Cable: plug face -> a sag/kink partway along -> the left edge.
    let cable_stroke = egui::Stroke::new(1.0f32, shell);
    let kink = egui::pos2(rect.left() + DW_ICON_SIZE.x * 0.35, rect.bottom() - 1.0);
    painter.line_segment([egui::pos2(plug.left(), rect.center().y), kink], cable_stroke);
    painter.line_segment([kink, egui::pos2(rect.left(), rect.center().y - DW_ICON_SIZE.y * 0.1)], cable_stroke);
    response
}

/// Status-bar RS-232 icon size: a DB-connector silhouette.
const RS232_ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 9.0);

/// Width of the pin field (the trapezoidal D-sub shield), excluding the ear
/// studs on each side — see [`rs232_icon`].
const RS232_PIN_FIELD_W: f32 = RS232_ICON_SIZE.x * 0.72;

/// Status-bar RS-232 (Deluxe RS-232 Program Pak) activity indicator (see
/// [`RS232_ICON_SIZE`]): red while a byte has recently gone out to or come
/// in from the host endpoint
/// ([`coco_core::rs232::DeluxeRS232::tx_bytes`]/`rx_bytes`), gray
/// otherwise. A trapezoid (top edge wider than the bottom, the classic
/// D-sub shield shape) carries three punched pin dots in an upper row and
/// two in a lower row, plus a small filled ear stud at each side (the
/// connector's mounting-screw bosses).
pub(crate) fn rs232_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, RS232_ICON_SIZE, active);

    // D-sub shield: a trapezoid, top edge the full pin-field width, bottom
    // edge narrower.
    let bottom_inset = RS232_PIN_FIELD_W * 0.15;
    let cx = rect.center().x;
    let half_top = RS232_PIN_FIELD_W / 2.0;
    let points = vec![
        egui::pos2(cx - half_top, rect.top()),
        egui::pos2(cx + half_top, rect.top()),
        egui::pos2(cx + half_top - bottom_inset, rect.bottom()),
        egui::pos2(cx - half_top + bottom_inset, rect.bottom()),
    ];
    painter.add(egui::Shape::convex_polygon(points, shell, egui::Stroke::NONE));

    // Ear studs: mounting-screw bosses at each side, at mid-height.
    let ear_r = RS232_ICON_SIZE.y * 0.16;
    painter.circle_filled(egui::pos2(rect.left() + ear_r, rect.center().y), ear_r, shell);
    painter.circle_filled(egui::pos2(rect.right() - ear_r, rect.center().y), ear_r, shell);

    // Pins: 3 over 2, punched into the shield.
    let pin_r = RS232_ICON_SIZE.y * 0.09;
    let upper_y = rect.center().y - RS232_ICON_SIZE.y * 0.16;
    let lower_y = rect.center().y + RS232_ICON_SIZE.y * 0.16;
    for dx in [-0.28, 0.0, 0.28] {
        painter.circle_filled(egui::pos2(cx + dx * RS232_PIN_FIELD_W, upper_y), pin_r, punch);
    }
    for dx in [-0.16, 0.16] {
        painter.circle_filled(egui::pos2(cx + dx * RS232_PIN_FIELD_W, lower_y), pin_r, punch);
    }
    response
}

/// Status-bar joystick icon size: an analog-stick silhouette.
const JOYSTICK_ICON_SIZE: egui::Vec2 = egui::vec2(11.0, 13.0);

/// Status-bar joystick activity indicator (see [`JOYSTICK_ICON_SIZE`]): red
/// while that port's source is actively being driven
/// ([`crate::joy::JoystickInputs::in_use`]), gray otherwise. A rounded base
/// rect, a stick line rising from the base's center, and a filled ball cap
/// on top — base, stick, and ball are all the same silhouette color (a real
/// stick reads as one continuous shape from above) — plus one small button
/// dot punched into the base's left corner.
pub(crate) fn joystick_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, JOYSTICK_ICON_SIZE, active);

    // Base: a rounded rect hugging the bottom.
    let base_h = JOYSTICK_ICON_SIZE.y * 0.4;
    let base = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - base_h),
        egui::vec2(JOYSTICK_ICON_SIZE.x, base_h),
    );
    painter.rect_filled(base, 1.5, shell);

    // Stick: rises from the base's top-center toward the icon's top.
    let ball = egui::pos2(base.center().x, rect.top() + JOYSTICK_ICON_SIZE.y * 0.12);
    let stick_stroke = egui::Stroke::new(2.0f32, shell);
    painter.line_segment([egui::pos2(base.center().x, base.top()), ball], stick_stroke);
    // Ball: the fire-button cap on top.
    painter.circle_filled(ball, JOYSTICK_ICON_SIZE.y * 0.10, shell);

    // Button: punched into the base's left corner.
    let button = egui::pos2(base.left() + base_h * 0.35, base.center().y);
    painter.circle_filled(button, base_h * 0.22, punch);
    response
}

/// Status-bar printer icon size: a dot-matrix printer silhouette (body plus
/// a sheet of paper feeding out its top).
const PRINTER_ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 13.0);

/// Height of the printer body rect, along the bottom of [`PRINTER_ICON_SIZE`].
const PRINTER_BODY_H: f32 = 7.0;

/// Size of the paper-sheet rect rising from the body's top.
const PRINTER_PAPER_SIZE: egui::Vec2 = egui::vec2(8.0, 6.0);

/// Status-bar printer activity indicator (see [`PRINTER_ICON_SIZE`]): red
/// while a byte has recently been decoded to the live sink
/// ([`coco_core::bitbanger::BitBanger::bytes_out`]), gray otherwise. The
/// silhouette is the union of the body rect and the paper rect (so they
/// read as one printer, not two overlapping shapes), with a 1px exit-slot
/// line punched where the paper meets the body and two small control-light
/// dots punched into the body's right side.
pub(crate) fn printer_icon(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, PRINTER_ICON_SIZE, active);

    // Body: the wide rect along the bottom.
    let body = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.bottom() - PRINTER_BODY_H),
        egui::vec2(PRINTER_ICON_SIZE.x, PRINTER_BODY_H),
    );
    painter.rect_filled(body, 1.5, shell);
    // Paper: the narrower rect rising from the body's top.
    let paper = egui::Rect::from_min_size(
        egui::pos2(rect.center().x - PRINTER_PAPER_SIZE.x / 2.0, body.top() - PRINTER_PAPER_SIZE.y),
        PRINTER_PAPER_SIZE,
    );
    painter.rect_filled(paper, 1.0, shell);

    // Exit slot: a thin punched line where the paper meets the body.
    let slot = egui::Rect::from_center_size(
        egui::pos2(paper.center().x, body.top()),
        egui::vec2(PRINTER_PAPER_SIZE.x * 0.8, 1.0),
    );
    painter.rect_filled(slot, 0.0, punch);

    // Control lights: two dots punched into the body's right side.
    let light_r = PRINTER_BODY_H * 0.12;
    for dy in [-PRINTER_BODY_H * 0.2, PRINTER_BODY_H * 0.2] {
        let light = egui::pos2(body.right() - PRINTER_BODY_H * 0.3, body.center().y + dy);
        painter.circle_filled(light, light_r, punch);
    }
    response
}

/// Status-bar keyboard-mode icon size: a keyboard silhouette.
const KEYBOARD_ICON_SIZE: egui::Vec2 = egui::vec2(15.0, 9.0);

/// Status-bar keyboard-mode indicator (see [`KEYBOARD_ICON_SIZE`]). Always
/// drawn in [`super::ICON_IDLE`] — "activity" doesn't mean anything for the
/// keyboard mode readout, this is decoration matching the other status-bar
/// entries, not a light. A rounded rect with two rows of punched key dots.
pub(crate) fn keyboard_icon(ui: &mut egui::Ui) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, KEYBOARD_ICON_SIZE, false);
    painter.rect_filled(rect, 1.5, shell);

    let key_r = KEYBOARD_ICON_SIZE.y * 0.11;
    for row_frac in [0.35, 0.68] {
        let y = rect.top() + KEYBOARD_ICON_SIZE.y * row_frac;
        for col in 0..5 {
            let x = rect.left() + KEYBOARD_ICON_SIZE.x * (0.14 + col as f32 * 0.18);
            painter.circle_filled(egui::pos2(x, y), key_r, punch);
        }
    }
    response
}

/// Status-bar cartridge icon size: a ROM-pak silhouette.
const CART_ICON_SIZE: egui::Vec2 = egui::vec2(13.0, 10.0);

/// Status-bar cartridge indicator (see [`CART_ICON_SIZE`]). Always drawn in
/// [`super::ICON_IDLE`] — see [`keyboard_icon`]'s doc comment on why this
/// isn't a light. The ROM-pak body rect has a narrower label hump on top
/// and two punched grip notches nicked into the bottom edge.
pub(crate) fn cart_icon(ui: &mut egui::Ui) -> egui::Response {
    let Icon { rect, shell, punch, painter, response } = begin_icon(ui, CART_ICON_SIZE, false);

    // Body: the lower, full-width part of the pak.
    let hump_h = CART_ICON_SIZE.y * 0.35;
    let body = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + hump_h),
        egui::vec2(CART_ICON_SIZE.x, CART_ICON_SIZE.y - hump_h),
    );
    painter.rect_filled(body, 1.0, shell);
    // Label hump: a narrower rect on top, like a pak's raised label area.
    let hump_w = CART_ICON_SIZE.x * 0.6;
    let hump = egui::Rect::from_min_size(
        egui::pos2(rect.center().x - hump_w / 2.0, rect.top()),
        egui::vec2(hump_w, hump_h),
    );
    painter.rect_filled(hump, 1.0, shell);

    // Grip notches: nicked into the body's bottom edge.
    let notch_w = CART_ICON_SIZE.x * 0.16;
    let notch_h = CART_ICON_SIZE.y * 0.14;
    for cx_frac in [0.28, 0.72] {
        let notch = egui::Rect::from_center_size(
            egui::pos2(rect.left() + CART_ICON_SIZE.x * cx_frac, rect.bottom()),
            egui::vec2(notch_w, notch_h * 2.0),
        );
        painter.rect_filled(notch, 0.0, punch);
    }
    response
}

/// Status-bar Multi-Pak Interface icon size: a card-cage silhouette.
const MPI_ICON_SIZE: egui::Vec2 = egui::vec2(15.0, 11.0);

/// Status-bar Multi-Pak Interface indicator (see [`MPI_ICON_SIZE`]). Always
/// drawn in [`super::ICON_IDLE`] — see [`keyboard_icon`]'s doc comment on
/// why this isn't a light. A box with four punched vertical slot lines, one
/// per cartridge slot the real FD-502 Multi-Pak exposes.
pub(crate) fn mpi_icon(ui: &mut egui::Ui) -> egui::Response {
    let Icon { rect, punch, painter, shell, response } = begin_icon(ui, MPI_ICON_SIZE, false);
    painter.rect_filled(rect, 1.5, shell);

    let slot_stroke = egui::Stroke::new(1.0f32, punch);
    let slot_top = rect.top() + MPI_ICON_SIZE.y * 0.2;
    let slot_bottom = rect.bottom() - MPI_ICON_SIZE.y * 0.2;
    for col in 0..4 {
        let x = rect.left() + MPI_ICON_SIZE.x * (0.18 + col as f32 * 0.22);
        painter.line_segment([egui::pos2(x, slot_top), egui::pos2(x, slot_bottom)], slot_stroke);
    }
    response
}
