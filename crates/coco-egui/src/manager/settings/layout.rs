//! Grouped, viewport-bounded presentation of the global settings draft.

use clap::ValueEnum;
use eframe::egui;

use super::{SettingsAction, SettingsDialog, SettingsTab};
use crate::cli::LogLevel;
use crate::config;
use crate::control::{DEFAULT_PORT, MCP_PATH};
use crate::manager::DETAIL_SECTION_GAP;

const DIALOG_WIDTH: f32 = 560.0;
const VIEWPORT_MARGIN: f32 = 24.0;
const DIALOG_INNER_MARGIN: i8 = 16;
const GROUP_INNER_MARGIN: i8 = 12;
const HEADING_GAP: f32 = 8.0;
const FOOTER_SEPARATOR_SPACING: f32 = 6.0;
const CONTENT_MAX_HEIGHT: f32 = 260.0;
const FIELD_WIDTH: f32 = 160.0;
const CONTROL_PORT_FIELD_WIDTH: f32 = 80.0;
const BUTTON_WIDTH: f32 = 80.0;
/// Disabling the server is handled by its checkbox.
const CONTROL_PORT_RANGE: std::ops::RangeInclusive<u16> = 1..=u16::MAX;
/// The persisted interval is nonzero.
const WELCOME_IMAGE_CYCLE_SECS_RANGE: std::ops::RangeInclusive<u32> = 1..=u32::MAX;
const CHECK_FOR_UPDATES_LABEL: &str = "Check for updates at startup";
const CHECK_FOR_UPDATES_HOVER: &str =
    "Ask GitHub for the latest release when CoCoVM starts. Takes effect at the next launch.";

const ASSETS_URL_HOVER: &str =
    "Download source for emulator assets. Leave empty to use the default.";
const DOWNLOAD_SOURCES_HELP: &str = "Use the default download sources, or enter a mirror URL.";

const EOU_URL_HOVER: &str = "Download source for the NitrOS-9 Ease of Use disk images. A \
    mirror must serve the same archive. Leave empty to use the default.";

pub(super) fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.style()).inner_margin(DIALOG_INNER_MARGIN)
}

impl SettingsDialog {
    pub(super) fn draw(&mut self, ui: &mut egui::Ui) -> SettingsAction {
        self.hotkey_editor.take_captured_key(ui);
        let viewport = ui.ctx().content_rect().size();
        let frame_margin = dialog_frame(ui.ctx()).total_margin().sum();
        let available = viewport - egui::Vec2::splat(VIEWPORT_MARGIN * 2.0) - frame_margin;
        let width = DIALOG_WIDTH.min(available.x.max(0.0));
        ui.set_width(width);
        let header_top = ui.cursor().min.y;
        ui.heading("Settings");
        ui.add_space(HEADING_GAP);
        self.draw_tabs(ui);
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(DETAIL_SECTION_GAP);
        let header_height = ui.cursor().min.y - header_top;
        let footer_height = DETAIL_SECTION_GAP * 2.0
            + FOOTER_SEPARATOR_SPACING
            + ui.spacing().interact_size.y
            + ui.spacing().item_spacing.y * 3.0;
        // A fixed viewport-bounded body avoids feeding the previous modal height
        // back into ScrollArea sizing, which moves controls across opening frames.
        let content_height =
            (available.y - header_height - footer_height).clamp(0.0, CONTENT_MAX_HEIGHT);
        egui::ScrollArea::vertical()
            .id_salt(("settings_content", self.tab))
            .min_scrolled_height(content_height)
            .max_height(content_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.push_id(self.tab, |ui| self.draw_tab(ui));
            });
        ui.add_space(DETAIL_SECTION_GAP);
        ui.add(egui::Separator::default().spacing(FOOTER_SEPARATOR_SPACING));
        ui.add_space(DETAIL_SECTION_GAP);
        self.draw_actions(ui)
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui) {
        let previous = self.tab;
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (SettingsTab::General, "General"),
                (SettingsTab::Hotkeys, "Hotkeys"),
                (SettingsTab::McpServer, "MCP server"),
                (SettingsTab::Advanced, "Advanced"),
                (
                    SettingsTab::Templates,
                    super::templates::TEMPLATES_TAB_LABEL,
                ),
            ] {
                ui.selectable_value(&mut self.tab, tab, label);
            }
        });
        if self.tab != previous {
            self.hotkey_editor.cancel_capture();
        }
    }

    fn draw_tab(&mut self, ui: &mut egui::Ui) {
        match self.tab {
            SettingsTab::General => {
                section(ui, "Appearance", |ui| {
                    ui.checkbox(&mut self.toolbar_icons_only, "Toolbar icons only");
                    ui.checkbox(&mut self.status_bar_icons_only, "Status bar icons only");
                });
                section(ui, "Welcome images", |ui| self.draw_welcome_images(ui));
                section(ui, "Updates", |ui| {
                    ui.checkbox(&mut self.check_for_updates, CHECK_FOR_UPDATES_LABEL)
                        .on_hover_text(CHECK_FOR_UPDATES_HOVER);
                });
            }
            SettingsTab::Hotkeys => self.hotkey_editor.draw(ui),
            SettingsTab::McpServer => self.draw_mcp_server(ui),
            SettingsTab::Advanced => self.draw_advanced(ui),
            SettingsTab::Templates => self.templates.draw(ui),
        }
    }

    fn draw_mcp_server(&mut self, ui: &mut egui::Ui) {
        section(ui, "Local connection", |ui| {
            ui.checkbox(&mut self.mcp_enabled, "Enable MCP server");
            let explanation = if self.mcp_enabled {
                "Allow MCP clients on this computer to control CoCoVM."
            } else {
                "Enable the server to let MCP clients on this computer control CoCoVM."
            };
            ui.add(egui::Label::new(egui::RichText::new(explanation).weak()).wrap());
            ui.add_space(HEADING_GAP);
            ui.add_enabled_ui(self.mcp_enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let label = ui.label("Port");
                    let size = egui::vec2(CONTROL_PORT_FIELD_WIDTH, ui.spacing().interact_size.y);
                    let port = ui
                        .add_sized(
                            size,
                            egui::DragValue::new(&mut self.control_port).range(CONTROL_PORT_RANGE),
                        )
                        .labelled_by(label.id);
                    let tooltip = format!(
                        "MCP client URL: http://{}:{}{MCP_PATH}",
                        std::net::Ipv4Addr::LOCALHOST,
                        self.control_port,
                    );
                    port.on_hover_text(&tooltip)
                        .on_disabled_hover_text(&tooltip);
                    ui.weak(format!("Default: {DEFAULT_PORT}"));
                });
            });
        });
    }

    fn draw_welcome_images(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.welcome_image_cycle, "Change welcome image every");
            ui.add_enabled(
                self.welcome_image_cycle,
                egui::DragValue::new(&mut self.welcome_image_cycle_secs)
                    .range(WELCOME_IMAGE_CYCLE_SECS_RANGE)
                    .suffix(" s"),
            );
        });
        ui.add_enabled(
            self.welcome_image_cycle,
            egui::Checkbox::new(&mut self.welcome_image_shuffle, "Shuffle welcome images"),
        );
    }

    fn draw_advanced(&mut self, ui: &mut egui::Ui) {
        section(ui, "Logging", |ui| self.draw_log_level(ui));
        section(ui, "Download sources", |ui| {
            ui.label(egui::RichText::new(DOWNLOAD_SOURCES_HELP).weak());
            ui.add_space(ui.spacing().item_spacing.y);
            download_source(
                ui,
                "Assets URL",
                &mut self.assets_url,
                crate::startup::DEFAULT_ASSETS_URL,
                ASSETS_URL_HOVER,
            );
            ui.add_space(DETAIL_SECTION_GAP);
            download_source(
                ui,
                "NitrOS-9 EOU URL",
                &mut self.eou_url,
                crate::nitros9::EOU_URL,
                EOU_URL_HOVER,
            );
        });
    }

    fn draw_log_level(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = ui.label("Log level");
            egui::ComboBox::from_id_salt("settings_log_level")
                .width(FIELD_WIDTH)
                .selected_text(config::log_level_name(self.log_level))
                .show_ui(ui, |ui| {
                    for level in LogLevel::value_variants() {
                        ui.selectable_value(
                            &mut self.log_level,
                            *level,
                            config::log_level_name(*level),
                        );
                    }
                })
                .response
                .labelled_by(label.id);
        });
    }

    fn draw_actions(&self, ui: &mut egui::Ui) -> SettingsAction {
        let mut action = SettingsAction::None;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = egui::vec2(BUTTON_WIDTH, ui.spacing().interact_size.y);
            if ui.add_sized(size, egui::Button::new("Save")).clicked() {
                action = SettingsAction::Save;
            }
            if ui.add_sized(size, egui::Button::new("Cancel")).clicked() {
                action = SettingsAction::Cancel;
            }
        });
        action
    }
}

fn section(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(GROUP_INNER_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong(title);
            ui.add_space(ui.spacing().item_spacing.y);
            contents(ui);
        });
    ui.add_space(DETAIL_SECTION_GAP);
}

fn download_source(ui: &mut egui::Ui, title: &str, draft: &mut String, default: &str, help: &str) {
    ui.push_id(title, |ui| {
        let label = ui.horizontal(|ui| {
            let label = ui.label(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let trimmed = draft.trim();
                let has_override = !trimmed.is_empty() && trimmed != default;
                let reset = ui.add_enabled(has_override, egui::Button::new("Use default"));
                reset.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        reset.enabled(),
                        format!("Use default {title}"),
                    )
                });
                if reset.clicked() {
                    *draft = default.to_string();
                }
                help_on_hover_or_focus(reset, &format!("Restore the default {title}: {default}"));
            });
            label
        });
        let field = ui
            .add(egui::TextEdit::singleline(draft).desired_width(f32::INFINITY))
            .labelled_by(label.inner.id);
        help_on_hover_or_focus(field, help);
    });
}

fn help_on_hover_or_focus(response: egui::Response, help: &str) {
    if response.has_focus() {
        egui::Tooltip::for_widget(&response).show(|ui| {
            ui.label(help);
        });
    } else {
        response.on_hover_text(help);
    }
}
