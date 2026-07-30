//! The manager window's toolbar: [`ManagerApp::draw_toolbar`] and the
//! icon-over-label button widget it's built from — the VirtualBox/Parallels
//! toolbar idiom (big glyph, small caption underneath) rather than egui's
//! default text-chip `ui.button`. Layout is New – Start – Suspend – Stop –
//! Reset – separator – Settings – separator – Help (user decision
//! 2026-07-29): the four transport tiles that used to live in the detail
//! pane's transport row and the bulk pane's transport row now live here
//! instead, acting on the current selection through the same
//! [`super::bulk::BulkAction`]/[`ManagerApp::apply_bulk`] dispatch the bulk
//! context menu uses — one code path, three surfaces.

use eframe::egui;

use super::bulk::BulkAction;
use super::{ManagerApp, PLAY_GLYPH, RESET_GLYPH, STOP_GLYPH, SUSPEND_GLYPH, SUSPEND_HOVER};
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

/// Horizontal breathing room on each side of a `ui.separator()` —
/// [`BUTTON_GAP`] is tuned for tile-to-tile spacing and reads as cramped
/// around a vertical rule.
const SEPARATOR_GAP: f32 = 6.0;

/// Toolbar glyphs, all verified present in egui's bundled emoji fonts
/// (NotoEmoji / emoji-icon-font — both monochrome, so they tint with the
/// widget text color like any label).
const NEW_ICON: &str = "➕";
const SETTINGS_ICON: &str = "⚙";
const HELP_ICON: &str = "❓";

/// Disabled-hover text shared by every transport tile when nothing at all
/// is selected — distinct from the flags-driven reason below it, so the
/// tile teaches *why* it's off from either starting point (selection-neutral
/// per user decision 2026-07-29, since the tiles now act on a selection of
/// any size rather than one fixed row).
const SELECT_A_MACHINE_HOVER: &str = "Select a machine first";
/// Disabled-hover text shared by Suspend and Reset — both gated on
/// [`super::bulk::BulkFlags::any_running`]. Reset's running-only rule
/// excludes a suspended machine deliberately: resetting a frozen machine's
/// live object without touching its `.ccstate` would silently desync the
/// two (same rationale as every other Reset control in the manager).
const NONE_RUNNING_HOVER: &str = "None of the selected machines are running";

const START_HOVER: &str = "Start or resume the selected machines that aren't already running";
const START_DISABLED_HOVER: &str = "The selected machines are already running";
const STOP_HOVER: &str = "Shut down the selected machines that are running or suspended";
const STOP_DISABLED_HOVER: &str = "None of the selected machines are running or suspended";
const RESET_HOVER: &str = "Press the reset button on the selected running machines";

impl ManagerApp {
    /// The manager actions row. "Settings"/"Help" are still inert
    /// scaffolding; the four transport tiles dispatch through
    /// [`ManagerApp::apply_bulk`] against every currently selected row
    /// (zero, one, or many — the same [`super::bulk::BulkFlags`] gating the
    /// bulk pane and bulk context menu use), so a single selected machine
    /// behaves exactly as the old per-row buttons did.
    pub(super) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        let indices: Vec<usize> = self.selection.iter().collect();
        let flags = self.bulk_flags(&indices);
        let empty_selection = indices.is_empty();
        let disabled_hover = |ineligible: &'static str| -> &'static str {
            if empty_selection { SELECT_A_MACHINE_HOVER } else { ineligible }
        };

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            // The shortcut shows on hover (inline shortcut text is a
            // menu-row convention, not a toolbar one).
            if toolbar_button(ui, NEW_ICON, "New", true)
                .on_hover_text(ui.ctx().format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT))
                .clicked()
            {
                self.create_machine_now();
            }

            if toolbar_button(ui, PLAY_GLYPH, "Start", flags.any_startable)
                .on_hover_text(START_HOVER)
                .on_disabled_hover_text(disabled_hover(START_DISABLED_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Play, &indices);
            }
            if toolbar_button(ui, SUSPEND_GLYPH, "Suspend", flags.any_running)
                .on_hover_text(SUSPEND_HOVER)
                .on_disabled_hover_text(disabled_hover(NONE_RUNNING_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Suspend, &indices);
            }
            if toolbar_button(ui, STOP_GLYPH, "Stop", flags.any_alive)
                .on_hover_text(STOP_HOVER)
                .on_disabled_hover_text(disabled_hover(STOP_DISABLED_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Stop, &indices);
            }
            if toolbar_button(ui, RESET_GLYPH, "Reset", flags.any_running)
                .on_hover_text(RESET_HOVER)
                .on_disabled_hover_text(disabled_hover(NONE_RUNNING_HOVER))
                .clicked()
            {
                self.apply_bulk(BulkAction::Reset, &indices);
            }

            toolbar_separator(ui);
            let _ = toolbar_button(ui, SETTINGS_ICON, "Settings", true);
            toolbar_separator(ui);
            let _ = toolbar_button(ui, HELP_ICON, "Help", true);
        });
    }
}

/// A vertical rule with [`SEPARATOR_GAP`] on each side, grouping the
/// toolbar into its New/transport, Settings, and Help clusters.
fn toolbar_separator(ui: &mut egui::Ui) {
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
fn toolbar_button(ui: &mut egui::Ui, icon: &str, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let enabled = ui.is_enabled();
        let (rect, response) = ui.allocate_exact_size(BUTTON_SIZE, egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
        });

        let visuals = if enabled {
            ui.style().interact(&response)
        } else {
            &ui.visuals().widgets.noninteractive
        };
        let painter = ui.painter();
        if enabled && (response.hovered() || response.is_pointer_button_down_on()) {
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
