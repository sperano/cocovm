//! On-screen keyboard-mapping overlay: draws the machine's own CoCo keyboard
//! — the real key plan, not a grid (`layout.rs` cites the sources) — and, in
//! positional mode, the host key that drives each CoCo key. Toggled from the
//! Keyboard menu or the Key layout hotkey (`hotkeys.rs`, F10 by default).
//!
//! Every legend here is either ASCII or a painted shape. That is deliberate:
//! the arrows and the host modifier symbols (⇧ ⌃ ⌥ ⌫) this window used to
//! print are absent from egui's bundled fonts, so they rendered as blank tofu
//! boxes — a legend nobody could read, which is the bug this rewrite fixes.

mod layout;

use eframe::egui;

use coco_core::{MachineVariant, keyboard};

use crate::hotkeys::{Hotkey, Hotkeys};
use crate::typeahead::{KeyModifiers, KeyTap};

use layout::{Cap, Dir, Legend, Slot};

/// Width of one key unit, cap plus the channel around it — `layout`'s widths
/// are pitches, not cap sizes. Wide enough for "BREAK" at [`WORD_SIZE`].
const UNIT_W: f32 = 40.0;
/// Pitch of one row, likewise: tall enough to stack the shifted legend, the
/// CoCo legend, and the host key without them touching.
const ROW_H: f32 = 48.0;

/// Half the dark channel between neighbouring caps: each cap is painted
/// inset by this much from its pitch, which is what makes the keys read as
/// separate without any layout spacing of egui's own.
const CAP_INSET: f32 = 1.5;

/// Inset from a cap's edge to its topmost/bottommost text.
const CAP_PAD: f32 = 3.0;
/// Corner rounding of a cap.
const CAP_RADIUS: f32 = 4.0;

// Legend sizes: the CoCo key is the cap's identity, the shifted legend and
// the host key are annotations on it.
/// The CoCo legend, when it is a single character.
const MAIN_SIZE: f32 = 17.0;
/// Word legends (BREAK, ENTER, SHIFT…) are printed smaller so they fit the
/// cap, exactly as they are on the real keyboard.
const WORD_SIZE: f32 = 9.5;
/// The shifted legend above the CoCo one.
const SHIFT_SIZE: f32 = 9.0;
/// The host key beneath it.
const HOST_SIZE: f32 = 9.5;

/// A legend of more than this many characters is a word, not a character.
const WORD_LEN: usize = 2;

/// Heights reserved above and below the CoCo legend for the two annotation
/// lines, when a cap has them.
const SHIFT_LINE_H: f32 = SHIFT_SIZE + 1.0;
const HOST_LINE_H: f32 = HOST_SIZE + 2.0;

/// Side of the square an arrow cap's painted triangle is inscribed in.
const ARROW_SIZE: f32 = 13.0;

/// Vertical breathing room around the keyboard block.
const SECTION_GAP: f32 = 8.0;

/// Joins the footer's hints on one line.
const HINT_SEPARATOR: &str = "   ·   ";

const HOST_GREEN: egui::Color32 = egui::Color32::from_rgb(0x30, 0xC0, 0x30);

/// Draw the keyboard-mapping window. `open` is toggled by the window's close
/// box; `variant` picks the key plan since the CoCo 3 added ALT/CTRL/F1/F2.
pub fn window(
    ctx: &egui::Context,
    open: &mut bool,
    symbolic: bool,
    variant: MachineVariant,
    enabled: bool,
    modifiers: &mut KeyModifiers,
    hotkeys: &Hotkeys,
) -> Vec<KeyTap> {
    let mut taps = Vec::new();
    egui::Window::new(crate::window_title(ctx, "CoCo Keyboard Mapping"))
        .open(open)
        .enabled(enabled)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.set_width(layout::width_units(variant) * UNIT_W);
            header(ui, symbolic);
            ui.add_space(SECTION_GAP);
            // No spacing of egui's own: layout widths already include the channel; per-item
            // spacing would break the arrow diamond.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                for row in layout::rows(variant) {
                    draw_row(ui, row, symbolic, modifiers, &mut taps);
                }
            });
            ui.add_space(SECTION_GAP);
            footer(ui, ctx, variant, hotkeys);
        });
    if !*open {
        *modifiers = KeyModifiers::default();
    }
    taps
}

fn header(ui: &mut egui::Ui, symbolic: bool) {
    ui.small(
        "Click a key to type. Click modifiers to toggle them; they stay on until toggled off.",
    );
    if symbolic {
        ui.label("Symbolic mode — type the character you want; it is sent as typed.");
        // Name the keys that produce no text — they're what people come to this window for.
        ui.label(
            egui::RichText::new("green = still pressed by position, even in this mode")
                .small()
                .color(HOST_GREEN),
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

fn footer(ui: &mut egui::Ui, ctx: &egui::Context, variant: MachineVariant, hotkeys: &Hotkeys) {
    // Shown in both modes — symbolic still routes these by position; spelled out since arrow
    // glyphs would render as tofu.
    let mut hints = String::from("Left arrow also on Backspace   ·   CLEAR also on `");
    if variant == MachineVariant::Coco3 {
        hints.push_str("   ·   F1/F2 may need Fn on a laptop");
    }
    ui.small(hints);
    let name = |hotkey: Hotkey| ctx.format_shortcut(&hotkey.shortcut());
    let mut hotkey_hints = vec![
        format!("{}: positional / symbolic", name(hotkeys.keyboard_mode)),
        format!("{}: show/hide this help", name(hotkeys.key_layout)),
    ];
    if cfg!(feature = "debug-ui") {
        hotkey_hints.push(format!("{}: debugger", name(hotkeys.debugger)));
    }
    ui.small(hotkey_hints.join(HINT_SEPARATOR));
    ui.small(crate::save_state::slot_shortcuts_hint(ctx, hotkeys));
}

fn draw_row(
    ui: &mut egui::Ui,
    row: layout::Row,
    symbolic: bool,
    modifiers: &mut KeyModifiers,
    taps: &mut Vec<KeyTap>,
) {
    ui.horizontal(|ui| {
        for slot in layout::slots(row) {
            match slot {
                Slot::Gap(units) => ui.add_space(units * UNIT_W),
                Slot::Cap(cap) => {
                    if let Some(tap) = draw_cap(ui, cap, symbolic, modifiers) {
                        taps.push(tap);
                    }
                }
            }
        }
    });
}

/// One key cap: shifted legend along the top, CoCo legend in the middle,
/// host key (positional mode) along the bottom — mirroring the real caps'
/// shifted-above-unshifted print.
fn draw_cap(
    ui: &mut egui::Ui,
    cap: &Cap,
    symbolic: bool,
    modifiers: &mut KeyModifiers,
) -> Option<KeyTap> {
    let (id, pitch_rect) = ui.allocate_space(egui::vec2(cap.width * UNIT_W, ROW_H));
    let rect = pitch_rect.shrink(CAP_INSET);
    let response = ui.interact(rect, id, egui::Sense::click());
    let modifier = match cap.pos() {
        keyboard::SHIFT => Some(&mut modifiers.shift),
        keyboard::CTRL => Some(&mut modifiers.ctrl),
        keyboard::ALT => Some(&mut modifiers.alt),
        _ => None,
    };
    let mut selected = modifier.as_deref().copied().unwrap_or(false);
    let mut tap = None;
    if response.clicked() {
        // Release any previous text field's focus as well as this keycap's.
        ui.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        if let Some(latched) = modifier {
            *latched = !*latched;
            selected = *latched;
        } else {
            tap = Some(KeyTap {
                pos: cap.pos(),
                modifiers: *modifiers,
            });
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            selected,
            cap.label(),
        )
    });
    let visuals = ui.style().interact_selectable(&response, selected);
    ui.painter().rect_filled(rect, CAP_RADIUS, visuals.bg_fill);
    paint_cap_legends(ui, cap, symbolic, rect);
    tap
}

fn paint_cap_legends(ui: &egui::Ui, cap: &Cap, symbolic: bool, rect: egui::Rect) {
    let visuals = ui.visuals();
    let painter = ui.painter();
    let cx = rect.center().x;
    let shift_h = if cap.shift.is_some() {
        SHIFT_LINE_H
    } else {
        0.0
    };
    // Symbolic mode drops the host line except on keys still routed by position (BREAK, CLEAR,
    // arrows, F1/F2).
    let show_host = !symbolic || cap.routed_in_symbolic();
    let host_h = if show_host { HOST_LINE_H } else { 0.0 };

    if let Some(shift) = cap.shift {
        painter.text(
            egui::pos2(cx, rect.top() + CAP_PAD),
            egui::Align2::CENTER_TOP,
            shift,
            egui::FontId::proportional(SHIFT_SIZE),
            visuals.weak_text_color(),
        );
    }

    // Centre the CoCo legend in whatever room the annotations leave, so all caps look like one row.
    let top = rect.top() + CAP_PAD + shift_h;
    let bottom = rect.bottom() - CAP_PAD - host_h;
    let middle = egui::pos2(cx, (top + bottom) / 2.0);
    paint_main_legend(painter, cap.main, middle, visuals.strong_text_color());

    if show_host {
        painter.text(
            egui::pos2(cx, rect.bottom() - CAP_PAD),
            egui::Align2::CENTER_BOTTOM,
            cap.host,
            egui::FontId::proportional(HOST_SIZE),
            HOST_GREEN,
        );
    }
}

fn paint_main_legend(
    painter: &egui::Painter,
    legend: Legend,
    middle: egui::Pos2,
    color: egui::Color32,
) {
    match legend {
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
                color,
            );
        }
        Legend::Arrow(dir) => paint_arrow(painter, middle, dir, color),
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

#[cfg(test)]
#[path = "kbd_help_test.rs"]
mod tests;
