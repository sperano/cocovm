use crate::*;

/// A window title styled uniformly across the app: sized to the button font
/// and strong (bold). Applied to every [`egui::Window`] title so they match.
/// We can't just resize `TextStyle::Heading` globally (egui's window-title
/// fallback) because content `ui.heading()` calls share that style.
pub(crate) fn window_title(ctx: &egui::Context, text: &str) -> egui::RichText {
    let size = ctx.style().text_styles[&egui::TextStyle::Button].size;
    egui::RichText::new(text).size(size).strong()
}

/// Fieldset-style titled group: a bordered box whose title interrupts the
/// top border — the classic Qt `QGroupBox` / HTML `<fieldset>` look, which
/// egui has no built-in equivalent for (`ui.group` puts the title *inside*
/// the box). Drawn as five bare line segments — the top edge split around
/// the title — rather than a background-colored rect painted over a full
/// border, so it renders correctly over any window/panel fill. Square
/// corners: erasing a rounded stroke under the title would need exactly
/// the background-fill hack this avoids. The box spans the full available
/// width (a fieldset that hugged its content would give every group a
/// different width), and the title is a real `Label` so it lands in the
/// AccessKit tree for screen readers and `ui_tests`.
///
/// Assumes a vertical host layout (the title-overhang spacer is vertical)
/// and a title narrower than the box — a wider one would collapse the
/// top-right border segment and clip mid-glyph.
pub(crate) fn titled_group<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    /// Title's x offset from the box's left corner.
    const TITLE_INDENT: f32 = 8.0;
    /// The title's breathing room inside the gap in the border.
    const TITLE_PAD: f32 = 4.0;
    /// Padding between the border and the contents.
    const INNER_MARGIN: i8 = 10;

    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    // Measured up front to size the gap; the visible title is the `put`
    // Label below, in the same Body font and strong color.
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(title.to_owned(), font, color));
    // Room above the box for the half of the title that overhangs the
    // border line.
    ui.add_space(galley.size().y / 2.0);
    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin::same(INNER_MARGIN))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui)
        });
    let rect = inner.response.rect;
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let gap_start = rect.left() + TITLE_INDENT;
    let gap_end = (gap_start + 2.0 * TITLE_PAD + galley.size().x).min(rect.right());
    let painter = ui.painter();
    painter.line_segment([rect.left_top(), egui::pos2(gap_start, rect.top())], stroke);
    painter.line_segment([egui::pos2(gap_end, rect.top()), rect.right_top()], stroke);
    painter.line_segment([rect.left_top(), rect.left_bottom()], stroke);
    painter.line_segment([rect.right_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.left_bottom(), rect.right_bottom()], stroke);
    let title_rect = egui::Rect::from_min_size(
        egui::pos2(gap_start + TITLE_PAD, rect.top() - galley.size().y / 2.0),
        galley.size(),
    );
    ui.put(
        title_rect,
        egui::Label::new(egui::RichText::new(title).strong()).selectable(false),
    );
    inner.inner
}

/// Drives the UI exposes. The FD-502 latch can address four, but real setups
/// were one or two — and the menu stays small.
pub(crate) const UI_DRIVES: usize = 2;

/// Status-bar drive activity indicator: a little 5¼" floppy jacket, red
/// while the drive is selected with its motor on ([`coco_core::fdc`]'s
/// `drive_active`, like a real drive's front-panel light), dim otherwise.
pub(crate) const DRIVE_ICON_SIZE: f32 = 11.0;

pub(crate) const DRIVE_ICON_ACTIVE: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x30, 0x30);

pub(crate) const DRIVE_ICON_IDLE: egui::Color32 = egui::Color32::from_gray(70);

/// Corner rounding of the jacket square.
pub(crate) const DRIVE_ICON_CORNER: f32 = 1.5;

/// Status-bar cassette activity indicator, the tape sibling of
/// [`DRIVE_ICON_SIZE`]'s floppy: shell proportions of a compact cassette
/// (wider than tall), red while the cassette relay is closed
/// (CLOAD/CSAVE/`MOTOR ON`), dim otherwise.
pub(crate) const TAPE_ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 10.0);

/// Corner rounding of the cassette shell.
pub(crate) const TAPE_ICON_CORNER: f32 = 1.5;

/// One status-bar cassette indicator (see [`TAPE_ICON_SIZE`]'s doc): the
/// shell with the two reel hubs punched out in the panel's background
/// color.
pub(crate) fn cassette_activity_light(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(TAPE_ICON_SIZE, egui::Sense::hover());
    let shell = if active { DRIVE_ICON_ACTIVE } else { DRIVE_ICON_IDLE };
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

/// One status-bar activity indicator (see [`DRIVE_ICON_SIZE`]'s doc): the
/// jacket square with the hub hole and the oblong head-access slot punched
/// out in the panel's background color — the 5¼" silhouette.
pub(crate) fn drive_activity_light(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(DRIVE_ICON_SIZE, DRIVE_ICON_SIZE),
        egui::Sense::hover(),
    );
    let jacket = if active { DRIVE_ICON_ACTIVE } else { DRIVE_ICON_IDLE };
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
