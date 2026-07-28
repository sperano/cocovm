//! The manager window's toolbar: [`ManagerApp::draw_toolbar`] and the
//! icon-over-label button widget it's built from — the VirtualBox/Parallels
//! toolbar idiom (big glyph, small caption underneath) rather than egui's
//! default text-chip `ui.button`.

use eframe::egui;

use super::ManagerApp;
use crate::new_vm;

/// Fixed footprint of one toolbar button. Wide enough for the longest
/// caption ("Settings" at [`LABEL_FONT_SIZE`]); tall enough for the icon
/// row plus the caption with [`ICON_TOP_PAD`]/[`LABEL_BOTTOM_PAD`] breathing
/// room. Fixed (not per-label sized) so the buttons read as one row of
/// equal tiles, the toolbar convention this module exists to reproduce.
const BUTTON_SIZE: egui::Vec2 = egui::vec2(64.0, 52.0);

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
/// default item spacing so the tiles read as one grouped toolbar.
const BUTTON_GAP: f32 = 2.0;

/// Toolbar glyphs, all verified present in egui's bundled emoji fonts
/// (NotoEmoji / emoji-icon-font — both monochrome, so they tint with the
/// widget text color like any label).
const NEW_ICON: &str = "➕";
const SETTINGS_ICON: &str = "⚙";
const HELP_ICON: &str = "❓";

impl ManagerApp {
    /// The manager actions row. "Settings"/"Help" are still inert
    /// scaffolding.
    pub(super) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            // The shortcut shows on hover (inline shortcut text is a
            // menu-row convention, not a toolbar one).
            if toolbar_button(ui, NEW_ICON, "New")
                .on_hover_text(ui.ctx().format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT))
                .clicked()
            {
                self.create_machine_now();
            }
            let _ = toolbar_button(ui, SETTINGS_ICON, "Settings");
            let _ = toolbar_button(ui, HELP_ICON, "Help");
        });
    }
}

/// One toolbar tile: `icon` large on top, `label` small underneath, with a
/// rounded highlight behind the whole tile on hover/press and no chrome at
/// rest (flat-toolbar idiom). Hand-painted because `egui::Button` can't mix
/// two font sizes in one label.
fn toolbar_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });

    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    if response.hovered() || response.is_pointer_button_down_on() {
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
}
