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
    // A bare child Ui at the title's exact rect — NOT `ui.put`: `put`
    // allocates its rect in the parent layout, and this rect sits *above*
    // the just-closed frame, so the parent's cursor would snap back up and
    // everything drawn after the group would overlap it. A child Ui still
    // registers the Label in the AccessKit tree (screen readers,
    // `ui_tests`) without touching the parent cursor.
    let mut title_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    title_ui.add(egui::Label::new(egui::RichText::new(title).strong()).selectable(false));
    inner.inner
}

/// Drives the UI exposes. The FD-502 latch can address four, but real setups
/// were one or two — and the menu stays small.
pub(crate) const UI_DRIVES: usize = 2;
