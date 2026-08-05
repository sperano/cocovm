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

// [`toolbar_button`], the icon-over-label toolbar tile, and its supporting
// glyphs/layout constants — the VirtualBox/Parallels toolbar idiom (big
// glyph, small caption underneath) rather than egui's default text-chip
// `ui.button`. Shared by the manager window's toolbar
// (`manager::toolbar::draw_toolbar`) and the VM window's toolbar
// (`chrome::toolbar::toolbar_ui`) — two independent `TopBottomPanel`s that
// both draw a row of these tiles.

/// Fixed footprint of one toolbar tile. Wide enough for the longest caption
/// ("Settings", the manager toolbar's widest label, at [`LABEL_FONT_SIZE`]);
/// tall enough for the icon row plus the caption with
/// [`ICON_TOP_PAD`]/[`LABEL_BOTTOM_PAD`] breathing room. Fixed (not
/// per-label sized) so the buttons read as one row of equal tiles, the
/// toolbar convention the tile widget reproduces.
pub(crate) const BUTTON_SIZE: egui::Vec2 = egui::vec2(64.0, 52.0);

/// Icon glyph size. Deliberately much larger than the caption — the icon is
/// the button's identity, the caption is the reminder.
const ICON_FONT_SIZE: f32 = 20.0;
const LABEL_FONT_SIZE: f32 = 11.0;

/// Gap from the button's top edge to the icon's top, and from the caption's
/// baseline box to the bottom edge.
const ICON_TOP_PAD: f32 = 5.0;
const LABEL_BOTTOM_PAD: f32 = 4.0;

/// Rounding of the hover/press highlight behind a button.
const BUTTON_CORNER_RADIUS: f32 = 6.0;

/// Horizontal gap between adjacent toolbar buttons — tighter than egui's
/// default item spacing so the tiles read as one grouped toolbar. Applied by
/// each caller (`ui.spacing_mut().item_spacing.x = BUTTON_GAP`), not by
/// [`toolbar_button`] itself: it's a property of the row a caller lays out,
/// not of one tile.
pub(crate) const BUTTON_GAP: f32 = 2.0;

/// Horizontal breathing room on each side of a `ui.separator()` —
/// [`BUTTON_GAP`] is tuned for tile-to-tile spacing and reads as cramped
/// around a vertical rule.
const SEPARATOR_GAP: f32 = 6.0;

/// Cassette-deck transport glyphs, shared by every surface that draws a
/// Start/Suspend/Stop/Reset control: the manager toolbar's tiles, the VM
/// window toolbar's tiles, and the manager's row/bulk context menus.
pub(crate) const PLAY_GLYPH: &str = "▶";
pub(crate) const SUSPEND_GLYPH: &str = "⏸";
pub(crate) const STOP_GLYPH: &str = "⏹";
pub(crate) const RESET_GLYPH: &str = "↻";

/// Caption text of the four transport tiles — shared by the manager
/// toolbar's tiles (`manager::toolbar::draw_toolbar`) and the VM window
/// toolbar's tiles (`chrome::toolbar::toolbar_ui`) so the two can't drift on
/// the one piece of tile identity the glyph consts above didn't already
/// cover. `ui_tests` still assert the raw string literals independently, to
/// pin the user-visible text rather than just this constant's own value.
pub(crate) const START_LABEL: &str = "Start";
pub(crate) const SUSPEND_LABEL: &str = "Suspend";
pub(crate) const STOP_LABEL: &str = "Stop";
pub(crate) const RESET_LABEL: &str = "Reset";

/// Hover text of the Suspend transport control everywhere it appears (the
/// manager toolbar tile, the manager's row/bulk context-menu items, and the
/// VM window's own Suspend tile) — the *heavy* freeze built on the
/// save-states engine.
pub(crate) const SUSPEND_HOVER: &str = "Suspend the machine — freeze it to disk; resume later, even after \
     quitting the manager.";

/// A vertical rule with [`SEPARATOR_GAP`] on each side, grouping a toolbar
/// into clusters (e.g. the manager toolbar's New/transport, Settings, and
/// Help clusters).
pub(crate) fn toolbar_separator(ui: &mut egui::Ui) {
    ui.add_space(SEPARATOR_GAP);
    ui.separator();
    ui.add_space(SEPARATOR_GAP);
}

/// One toolbar tile: `icon` large on top, `label` small underneath, with a
/// rounded highlight behind the whole tile on hover/press and no chrome at
/// rest (flat-toolbar idiom). Hand-painted because `egui::Button` can't mix
/// two font sizes in one label — which also means the disabled look doesn't
/// come for free the way `ui.add_enabled(Button::new(..))` gets it, so this
/// wraps its painting in [`egui::Ui::add_enabled_ui`]: that's what gives a
/// disabled tile the correct `Response::enabled()` (so `on_hover_text`/
/// `on_disabled_hover_text` pick the right one and a click can't sneak
/// through — `Ui::interact`'s enabled flag is what the input layer actually
/// gates on, not the `Sense` alone), while the noninteractive-visuals text
/// color and skipped hover fill below supply the grayed-out paint.
pub(crate) fn toolbar_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    enabled: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        // Not necessarily the same as the `enabled` parameter above:
        // `Ui::is_enabled` also ANDs in the parent's enabledness, so this is
        // load-bearing for a tile drawn inside an already-disabled parent.
        let effective_enabled = ui.is_enabled();
        let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, effective_enabled, label)
        });

        let visuals = if effective_enabled {
            ui.style().interact(&response)
        } else {
            &ui.visuals().widgets.noninteractive
        };
        let painter = ui.painter();
        if effective_enabled && (response.hovered() || response.is_pointer_button_down_on()) {
            painter.rect_filled(rect, BUTTON_CORNER_RADIUS, visuals.weak_bg_fill);
        }
        painter.text(
            egui::pos2(rect.center().x, rect.top() + ICON_TOP_PAD),
            egui::Align2::CENTER_TOP,
            icon,
            egui::FontId::proportional(ICON_FONT_SIZE),
            visuals.text_color(),
        );
        painter.text(
            egui::pos2(rect.center().x, rect.bottom() - LABEL_BOTTOM_PAD),
            egui::Align2::CENTER_BOTTOM,
            label,
            egui::FontId::proportional(LABEL_FONT_SIZE),
            visuals.text_color(),
        );
        response
    })
    .inner
}
