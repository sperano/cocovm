use crate::*;

/// A window title styled uniformly across the app: sized to the button font
/// and strong. Can't resize `TextStyle::Heading` globally — content
/// `ui.heading()` calls share that style.
pub(crate) fn window_title(ctx: &egui::Context, text: &str) -> egui::RichText {
    let size = ctx.style().text_styles[&egui::TextStyle::Button].size;
    egui::RichText::new(text).size(size).strong()
}

/// Padding between every [`titled_group`]'s border and its contents.
const TITLED_GROUP_PADDING: i8 = 16;

/// Fieldset-style titled group: a bordered box whose title interrupts the
/// top border (Qt `QGroupBox`/HTML `<fieldset>`, for which egui has no
/// built-in equivalent). Assumes a vertical host layout and a title
/// narrower than the box — a wider one clips mid-glyph.
pub(crate) fn titled_group<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    /// The title's breathing room inside the gap in the border.
    const TITLE_PAD: f32 = 4.0;
    /// The gap's x offset from the box's left corner; lines the title text up with the contents.
    const TITLE_INDENT: f32 = TITLED_GROUP_PADDING as f32 - TITLE_PAD;

    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    // Measured up front to size the gap; the visible title is the Label drawn below, in the
    // same Body font and strong color.
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(title.to_owned(), font, color));
    // Room above the box for the half of the title that overhangs the border line.
    ui.add_space(galley.size().y / 2.0);
    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin::same(TITLED_GROUP_PADDING))
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
    // A bare child Ui at the title's rect, not `ui.put`: `put` would snap the parent's cursor
    // back up and let later content overlap it.
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

/// Tile footprint when the caption is hidden: the glyph with equal padding.
pub(crate) const ICON_ONLY_BUTTON_SIZE: egui::Vec2 = egui::vec2(
    ICON_FONT_SIZE + 2.0 * ICON_ONLY_PAD,
    ICON_FONT_SIZE + 2.0 * ICON_ONLY_PAD,
);

/// Icon glyph size. Deliberately much larger than the caption — the icon is
/// the button's identity, the caption is the reminder.
const ICON_FONT_SIZE: f32 = 20.0;
const ICON_ONLY_PAD: f32 = 6.0;
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

/// Width a toolbar separator's vertical rule allocates — egui's
/// `Separator` default, named so [`toolbar_separator_width`] can count it.
const SEPARATOR_SPACING: f32 = 6.0;

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
/// the one piece of tile identity the earlier glyph consts didn't already
/// cover. `ui_tests` still assert the raw string literals independently, to
/// pin the user-visible text rather than this constant's own value.
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
/// into clusters.
pub(crate) fn toolbar_separator(ui: &mut egui::Ui) {
    ui.add_space(SEPARATOR_GAP);
    ui.add(egui::Separator::default().spacing(SEPARATOR_SPACING));
    ui.add_space(SEPARATOR_GAP);
}

/// Horizontal room [`toolbar_separator`] takes in a row laid out with `ui`'s
/// item spacing: the two gaps (`add_space` adds no item spacing) plus the
/// rule and the item spacing after it.
pub(crate) fn toolbar_separator_width(ui: &egui::Ui) -> f32 {
    2.0 * SEPARATOR_GAP + SEPARATOR_SPACING + ui.spacing().item_spacing.x
}

/// Horizontal room one [`toolbar_button`] takes in a row laid out with `ui`'s
/// item spacing: its fixed footprint plus the item spacing after it.
pub(crate) fn toolbar_button_width(ui: &egui::Ui, icons_only: bool) -> f32 {
    tile_size(icons_only).x + ui.spacing().item_spacing.x
}

/// [`BUTTON_SIZE`], or [`ICON_ONLY_BUTTON_SIZE`] when the caption is hidden.
pub(crate) fn tile_size(icons_only: bool) -> egui::Vec2 {
    if icons_only {
        ICON_ONLY_BUTTON_SIZE
    } else {
        BUTTON_SIZE
    }
}

/// One toolbar tile: `icon` large on top, `label` small underneath, with a
/// hover/press highlight. Hand-painted because `egui::Button` can't mix two
/// font sizes; wrapped in [`egui::Ui::add_enabled_ui`] so disabled tiles
/// still get a correct `Response::enabled()`.
///
/// `icons_only` (the `toolbar_icons_only` global setting, `config.rs`) draws
/// the icon alone, centered in a square [`ICON_ONLY_BUTTON_SIZE`] tile, and
/// moves `label` into hover text instead of painting it. The accessible name
/// is always `label` regardless, so `kittest`'s `get_by_label` keeps
/// resolving tiles the same way in both modes.
pub(crate) fn toolbar_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    enabled: bool,
    icons_only: bool,
) -> egui::Response {
    let response = toolbar_tile(ui, icon, label, label, enabled, icons_only);
    if icons_only {
        // Both tooltips, not just the enabled one: `on_hover_text` only fires when
        // enabled, so a disabled tile (the manager's four transport tiles at first
        // launch, before anything is selected) would otherwise show only the caller's
        // disabled-reason text and never say which tile it is. Same per-widget
        // `tooltip_count` stacking mechanism as `on_hover_text`/`on_disabled_hover_text`
        // chained at the call site, so the label lands first in either stack.
        response.on_hover_text(label).on_disabled_hover_text(label)
    } else {
        response
    }
}

/// [`toolbar_button`]'s painting with an accessible `name` that can say
/// more than the `caption` ("Save to State 1" under a "Save" caption), and
/// no tooltip of its own: the caller's hover texts must name the tile, since
/// icons-only mode paints no caption at all.
pub(crate) fn toolbar_tile(
    ui: &mut egui::Ui,
    icon: &str,
    caption: &str,
    name: &str,
    enabled: bool,
    icons_only: bool,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        // Not necessarily the same as `enabled`: `is_enabled` also ANDs in the parent's
        // enabledness.
        let effective_enabled = ui.is_enabled();
        let (rect, response) = ui.allocate_exact_size(tile_size(icons_only), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, effective_enabled, name)
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
        if icons_only {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(ICON_FONT_SIZE),
                visuals.text_color(),
            );
        } else {
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
                caption,
                egui::FontId::proportional(LABEL_FONT_SIZE),
                visuals.text_color(),
            );
        }
        response
    })
    .inner
}
