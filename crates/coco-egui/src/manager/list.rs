//! The machine-list panel: [`ManagerApp::draw_machine_list`] and one row's
//! worth of drawing/interaction ([`ManagerApp::draw_machine_row`]), plus the
//! row-thumbnail rendering helpers only this panel needs.

use eframe::egui;

use super::bulk::BulkAction;
use super::{
    CocoApp, ManagerApp, ROW_CORNER_RADIUS, ROW_MARGIN, THUMBNAIL_ASPECT, THUMBNAIL_CORNER_RADIUS,
    THUMBNAIL_PLACEHOLDER_FILL, vm_status_label,
};
use crate::new_vm;
use crate::widgets::SUSPEND_HOVER;

impl ManagerApp {
    /// Left panel: the machine list. `ui.set_min_width` keeps the
    /// `SidePanel`'s divider draggable even when the list is empty.
    pub(super) fn draw_machine_list(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        if !self.entries.is_empty() {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for i in 0..self.entries.len() {
                    self.draw_machine_row(ui, i);
                }
            });
        }
        self.deselect_on_empty_click(ui);
    }

    /// The panel space left below the last row: clicking it clears the
    /// selection, bringing the photo pane back. The edit state is dropped
    /// too, same as switching rows.
    fn deselect_on_empty_click(&mut self, ui: &mut egui::Ui) {
        let remaining = ui.available_size_before_wrap();
        if remaining.y <= 0.0 {
            return;
        }
        let (_, response) = ui.allocate_exact_size(remaining, egui::Sense::click());
        if response.clicked() && !self.selection.is_empty() {
            self.selection.clear();
            self.edit = None;
            self.save_error = None;
        }
    }

    /// One machine-list row: thumbnail + name/subtitle/status. Clicking
    /// anywhere in the row selects it (`ui.interact` over the frame's rect).
    pub(super) fn draw_machine_row(&mut self, ui: &mut egui::Ui, i: usize) {
        // A stopped machine's saved preview, if any, loads (once) before the row draws.
        self.ensure_row_thumbnail(&ui.ctx().clone(), i);
        let selected = self.selection.contains(i);
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame_rect = egui::Frame::new()
            .fill(fill)
            .inner_margin(ROW_MARGIN)
            .corner_radius(ROW_CORNER_RADIUS)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let content_height = row_content_height(ui);
                    // Preview priority: live VM framebuffer, else a window-closed Suspended machine's saved thumbnail, else the black placeholder.
                    let entry = &self.entries[i];
                    let texture = entry
                        .vm
                        .as_deref()
                        .and_then(CocoApp::framebuffer_texture)
                        .or(entry.thumbnail.as_ref().filter(|_| entry.suspended));
                    draw_row_thumbnail(ui, content_height, texture);

                    let def = &self.entries[i].def;
                    let config = def
                        .to_machine_config()
                        .expect("list entries are validated on load/save");
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&def.name).strong());
                        ui.label(format!(
                            "{} · {}",
                            crate::machine_label(config.variant),
                            new_vm::ram_label(config.memory),
                        ));
                        ui.weak(vm_status_label(&self.entries[i]));
                    });
                });
            })
            .response
            .rect;

        let click_id = ui.id().with(("machine_row", i));
        let response = ui.interact(frame_rect, click_id, egui::Sense::click());
        if response.clicked() {
            self.apply_row_click(ui, i);
        }
        self.draw_row_context_menu(response, i);
    }

    /// A plain click selects `i` alone; Shift extends/replaces the
    /// selection from the current anchor; Cmd/Ctrl flips `i`'s own
    /// membership, leaving the rest as-is.
    fn apply_row_click(&mut self, ui: &egui::Ui, i: usize) {
        let modifiers = ui.input(|input| input.modifiers);
        if modifiers.shift {
            let anchor = self.selection.anchor().unwrap_or(i);
            self.selection.select_range(anchor, i);
        } else if modifiers.command {
            self.selection.toggle(i);
        } else {
            self.selection.set_single(i);
        }
        self.on_selection_changed();
    }

    /// ⌘A/Ctrl+A ([`super::ManagerApp::update`]'s shortcut handler): select
    /// every row.
    pub(super) fn select_all_rows(&mut self) {
        self.selection.select_all(self.entries.len());
        self.on_selection_changed();
    }

    /// After any selection-changing operation: clear the stale save error,
    /// and drop `edit` whenever the result isn't exactly one row.
    fn on_selection_changed(&mut self) {
        self.save_error = None;
        if self.selection.len() != 1 {
            self.edit = None;
        }
    }

    /// Per-row context menu: the single-row menu for a plain selection, or
    /// when `i` sits outside the current multi-selection; the bulk menu
    /// when `i` is one of several selected rows. Right-click never itself
    /// moves the selection cue.
    fn draw_row_context_menu(&mut self, response: egui::Response, i: usize) {
        if self.selection.len() > 1 && self.selection.contains(i) {
            self.draw_bulk_row_context_menu(response);
        } else {
            self.draw_single_row_context_menu(response, i);
        }
    }

    /// The single-machine context menu. One exception to "right-click never
    /// selects": [`Self::select_row_on_error`] selects the row when a
    /// lifecycle action failed, since the error renders only in the detail pane.
    fn draw_single_row_context_menu(&mut self, response: egui::Response, i: usize) {
        response.context_menu(|ui| {
            // Same enablement as the toolbar; Start/Resume is one item whose label follows state.
            let suspended = self.entries[i].suspended;
            let running = self.entries[i].is_running();
            let start_label = if suspended { "Resume" } else { "Start" };
            if ui
                .add_enabled(!running, egui::Button::new(start_label))
                .clicked()
            {
                if suspended {
                    self.resume_vm(i);
                } else {
                    self.start_vm(i);
                }
                self.select_row_on_error(i);
                ui.close();
            }
            if ui
                .add_enabled(running, egui::Button::new("Suspend"))
                .on_hover_text(SUSPEND_HOVER)
                .clicked()
            {
                self.suspend_vm(i);
                self.select_row_on_error(i);
                ui.close();
            }
            if ui
                .add_enabled(running, egui::Button::new("Reset"))
                .clicked()
            {
                if let Some(vm) = self.entries[i].vm.as_mut() {
                    vm.machine.reset();
                }
                ui.close();
            }
            if ui
                .add_enabled(self.entries[i].is_alive(), egui::Button::new("Stop"))
                .clicked()
            {
                self.stop_vm(i);
                self.select_row_on_error(i);
                ui.close();
            }
            ui.separator();
            if ui.button("Show config").clicked() {
                self.selection.set_single(i);
                self.save_error = None;
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                self.pending_delete = vec![self.entries[i].slug.clone()];
                ui.close();
            }
        });
    }

    /// The multi-selection context menu: the same four transport actions as
    /// the toolbar, applied to every selected row, plus a bulk "Delete…".
    /// The picked action is applied only after the menu closure returns.
    fn draw_bulk_row_context_menu(&mut self, response: egui::Response) {
        let indices: Vec<usize> = self.selection.iter().collect();
        let flags = self.bulk_flags(&indices);
        let mut picked = None;
        response.context_menu(|ui| {
            if ui
                .add_enabled(flags.any_startable, egui::Button::new("Start"))
                .clicked()
            {
                picked = Some(BulkAction::Play);
                ui.close();
            }
            if ui
                .add_enabled(flags.any_running, egui::Button::new("Suspend"))
                .on_hover_text(SUSPEND_HOVER)
                .clicked()
            {
                picked = Some(BulkAction::Suspend);
                ui.close();
            }
            if ui
                .add_enabled(flags.any_running, egui::Button::new("Reset"))
                .clicked()
            {
                picked = Some(BulkAction::Reset);
                ui.close();
            }
            if ui
                .add_enabled(flags.any_alive, egui::Button::new("Stop"))
                .clicked()
            {
                picked = Some(BulkAction::Stop);
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                self.pending_delete = indices
                    .iter()
                    .map(|&i| self.entries[i].slug.clone())
                    .collect();
                ui.close();
            }
        });
        if let Some(action) = picked {
            self.apply_bulk(action, &indices);
        }
    }

    /// After a context-menu lifecycle action: if it recorded a launch
    /// error, select the row so the detail pane shows why nothing happened.
    fn select_row_on_error(&mut self, i: usize) {
        if self.entries[i].launch_error.is_some() {
            self.selection.set_single(i);
            self.save_error = None;
        }
    }
}

/// Height the row's text column (name/subtitle/status) will render at, used
/// to size the thumbnail to reach the same bottom edge. Computed from text
/// metrics up front, since the thumbnail is placed before the text column's
/// actual height is known.
fn row_content_height(ui: &egui::Ui) -> f32 {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
    let spacing = ui.spacing().item_spacing.y;
    line_height * 3.0 + spacing * 2.0
}

/// One list row's thumbnail: the resolved preview `texture`, sized to
/// `height` tall at the fixed [`THUMBNAIL_ASPECT`]. A paused VM's texture
/// simply stops changing, freezing the thumbnail on its last frame; with no
/// texture, just the placeholder shows.
fn draw_row_thumbnail(
    ui: &mut egui::Ui,
    height: f32,
    texture: Option<&egui::TextureHandle>,
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(height * THUMBNAIL_ASPECT, height),
        egui::Sense::hover(),
    );

    let painter = ui.painter();
    painter.rect_filled(rect, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_PLACEHOLDER_FILL);
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    rect
}
