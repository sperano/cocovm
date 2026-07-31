//! On-screen keyboard-mapping overlay: draws the machine's own CoCo keyboard
//! — the real key plan, not a grid (`layout.rs` cites the sources) — and, in
//! positional mode, the host key that drives each CoCo key. Toggled from the
//! Keyboard menu or F10.
//!
//! Every legend here is either ASCII or a painted shape. That is deliberate:
//! the arrows and the host modifier symbols (⇧ ⌃ ⌥ ⌫) this window used to
//! print are absent from egui's bundled fonts, so they rendered as blank tofu
//! boxes — a legend nobody could read, which is the bug this rewrite fixes.

use eframe::egui;

use coco_core::MachineVariant;

use layout::{Cap, Dir, Legend, Slot};

mod layout;

/// Pitch of one key unit and of one row — cap plus the channel around it,
/// since `layout`'s widths are pitches. A cap is wide enough for "BREAK" at
/// [`WORD_SIZE`] and tall enough to stack the shifted legend, the CoCo
/// legend, and the host key without them touching.
const UNIT_W: f32 = 40.0;
const ROW_H: f32 = 48.0;

/// Half the dark channel between neighbouring caps: each cap is painted
/// inset by this much from its pitch, which is what makes the keys read as
/// separate without any layout spacing of egui's own.
const CAP_INSET: f32 = 1.5;

/// Inset from a cap's edge to its topmost/bottommost text.
const CAP_PAD: f32 = 3.0;
const CAP_RADIUS: f32 = 4.0;

/// Legend sizes: the CoCo key is the cap's identity, the shifted legend and
/// the host key are annotations on it.
const MAIN_SIZE: f32 = 17.0;
/// Word legends (BREAK, ENTER, SHIFT…) are printed smaller so they fit the
/// cap, exactly as they are on the real keyboard.
const WORD_SIZE: f32 = 9.5;
const SHIFT_SIZE: f32 = 9.0;
const HOST_SIZE: f32 = 9.5;

/// A legend of more than this many characters is a word, not a character.
const WORD_LEN: usize = 2;

/// Height reserved under the CoCo legend for the host-key line.
const HOST_LINE_H: f32 = HOST_SIZE + 2.0;

/// Side of an arrow cap's painted triangle.
const ARROW_SIZE: f32 = 13.0;

/// Vertical breathing room around the keyboard block.
const SECTION_GAP: f32 = 8.0;

const HOST_GREEN: egui::Color32 = egui::Color32::from_rgb(0x30, 0xC0, 0x30);

/// Draw the keyboard-mapping window. `open` is toggled by the window's close
/// box; `variant` picks which machine's key plan to draw, since the CoCo 3
/// added ALT, CTRL, F1 and F2 to the CoCo 1/2's keyboard.
pub fn window(ctx: &egui::Context, open: &mut bool, symbolic: bool, variant: MachineVariant) {
    egui::Window::new(crate::window_title(ctx, "CoCo Keyboard Mapping"))
        .open(open)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.set_width(layout::width_units(variant) * UNIT_W);
            header(ui, symbolic);
            ui.add_space(SECTION_GAP);
            // No spacing of egui's own: `layout`'s widths are pitches that
            // already include the channel between caps, and rows carry
            // different numbers of caps — per-item spacing would push each
            // row out by a different amount and break the arrow diamond.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                for row in layout::rows(variant) {
                    draw_row(ui, row, symbolic);
                }
            });
            ui.add_space(SECTION_GAP);
            footer(ui, ctx, symbolic);
        });
}

fn header(ui: &mut egui::Ui, symbolic: bool) {
    if symbolic {
        ui.label("Symbolic mode — type the character you want; it is sent as typed.");
        ui.label(
            egui::RichText::new(
                "The keys below are the CoCo's own; in this mode you don't have to \
                 match its layout.",
            )
            .small()
            .weak(),
        );
    } else {
        ui.label("Positional mode — each host key acts as the CoCo key in the same slot.");
        ui.label(
            egui::RichText::new("green = the key to press on your keyboard")
                .small()
                .color(HOST_GREEN),
        );
    }
}

fn footer(ui: &mut egui::Ui, ctx: &egui::Context, symbolic: bool) {
    if !symbolic {
        // Spelled out, not drawn as arrows: this line is plain text, and the
        // arrow codepoints would come out as tofu boxes here (module doc).
        ui.small(
            "Left arrow also on Backspace   ·   CLEAR also on `   ·   \
             F1/F2 may need Fn on a laptop",
        );
    }
    ui.small("F12: positional / symbolic   ·   F10: show/hide this help");
    ui.small(crate::save_state::slot_shortcuts_hint(ctx));
}

fn draw_row(ui: &mut egui::Ui, row: layout::Row, symbolic: bool) {
    ui.horizontal(|ui| {
        for slot in layout::slots(row) {
            match slot {
                Slot::Gap(units) => ui.add_space(units * UNIT_W),
                Slot::Cap(cap) => draw_cap(ui, cap, symbolic),
            }
        }
    });
}

/// One key cap: the shifted legend small along the top, the CoCo legend large
/// in the middle, and — in positional mode — the host key in green along the
/// bottom. The two-legend stack mirrors the real caps, which print the
/// shifted character above the unshifted one.
fn draw_cap(ui: &mut egui::Ui, cap: &Cap, symbolic: bool) {
    let pitch = egui::vec2(cap.width * UNIT_W, ROW_H);
    let (slot, _) = ui.allocate_exact_size(pitch, egui::Sense::hover());
    // The cap is drawn inside its pitch, leaving the channel that separates
    // it from its neighbours (see `CAP_INSET`).
    let rect = slot.shrink(CAP_INSET);
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, CAP_RADIUS, visuals.widgets.inactive.bg_fill);

    let cx = rect.center().x;
    let shift_h = if cap.shift.is_empty() {
        0.0
    } else {
        SHIFT_SIZE + 1.0
    };
    let host_h = if symbolic { 0.0 } else { HOST_LINE_H };

    if !cap.shift.is_empty() {
        painter.text(
            egui::pos2(cx, rect.top() + CAP_PAD),
            egui::Align2::CENTER_TOP,
            cap.shift,
            egui::FontId::proportional(SHIFT_SIZE),
            visuals.weak_text_color(),
        );
    }

    // Centre the CoCo legend in whatever vertical room the two annotations
    // leave, so a plain letter cap and a shifted digit cap still look like
    // one row of keys.
    let top = rect.top() + CAP_PAD + shift_h;
    let bottom = rect.bottom() - CAP_PAD - host_h;
    let middle = egui::pos2(cx, (top + bottom) / 2.0);
    match cap.main {
        Legend::Text(text) => {
            let size = if text.chars().count() > WORD_LEN {
                WORD_SIZE
            } else {
                MAIN_SIZE
            };
            painter.text(
                middle,
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(size),
                visuals.strong_text_color(),
            );
        }
        Legend::Arrow(dir) => paint_arrow(painter, middle, dir, visuals.strong_text_color()),
    }

    if !symbolic {
        painter.text(
            egui::pos2(cx, rect.bottom() - CAP_PAD),
            egui::Align2::CENTER_BOTTOM,
            cap.host,
            egui::FontId::proportional(HOST_SIZE),
            HOST_GREEN,
        );
    }
}

/// An arrow cap's legend, as a filled triangle. Painted rather than written
/// because egui's bundled fonts have no arrow glyphs (see the module doc).
fn paint_arrow(painter: &egui::Painter, center: egui::Pos2, dir: Dir, color: egui::Color32) {
    let h = ARROW_SIZE / 2.0;
    let (x, y) = (center.x, center.y);
    let points = match dir {
        Dir::Up => vec![
            egui::pos2(x, y - h),
            egui::pos2(x - h, y + h),
            egui::pos2(x + h, y + h),
        ],
        Dir::Down => vec![
            egui::pos2(x, y + h),
            egui::pos2(x + h, y - h),
            egui::pos2(x - h, y - h),
        ],
        Dir::Left => vec![
            egui::pos2(x - h, y),
            egui::pos2(x + h, y + h),
            egui::pos2(x + h, y - h),
        ],
        Dir::Right => vec![
            egui::pos2(x + h, y),
            egui::pos2(x - h, y - h),
            egui::pos2(x - h, y + h),
        ],
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}
