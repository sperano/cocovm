//! The machine-list panel: [`ManagerApp::draw_machine_list`] and one row's
//! worth of drawing/interaction ([`ManagerApp::draw_machine_row`]). The
//! preview rendering itself is shared with the detail pane
//! (`manager::thumbnails`).

use eframe::egui;

use super::bulk::BulkAction;
use super::selection::Step;
use super::sort::SortKey;
use super::{
    ManagerApp, ROW_CORNER_RADIUS, ROW_MARGIN, THUMBNAIL_ASPECT, thumbnails, vm_status_label,
};
use crate::new_vm;
use crate::widgets::SUSPEND_HOVER;

/// Saved previews are prepared for the visible rows plus this many rows on
/// either side. This hides decode latency during ordinary wheel scrolling
/// without making filesystem work depend on the full library size.
const THUMBNAIL_NEAR_RANGE_ROWS: usize = 4;

/// Select every row (⌘A/Ctrl+A). Consumed only under [`list_has_keyboard`],
/// so the detail pane's text fields keep their native select-all.
const SELECT_ALL_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A);

/// The arrow keys that walk the machine list, under the same guard as
/// [`SELECT_ALL_SHORTCUT`].
const LIST_STEP_KEYS: [(egui::Key, Step); 2] = [
    (egui::Key::ArrowUp, Step::Up),
    (egui::Key::ArrowDown, Step::Down),
];

const SORT_CONTROLS_TOP_INSET: f32 = 4.0;
const SORT_ARROW_HALF_LENGTH: f32 = 5.0;
const SORT_ARROW_HEAD_LENGTH: f32 = 3.0;
const SORT_ARROW_STROKE_WIDTH: f32 = 1.5;

impl ManagerApp {
    /// ⌘A/Ctrl+A selects every row, ↑/↓ move the selection — only while
    /// the machine list has the keyboard ([`list_has_keyboard`]).
    pub(super) fn handle_list_shortcuts(&mut self, ctx: &egui::Context) {
        if !list_has_keyboard(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SELECT_ALL_SHORTCUT)) {
            self.select_all_rows();
        }
        for (key, step) in LIST_STEP_KEYS {
            // `consume_key` alone ignores extra Shift/Alt.
            let bare_key = |i: &mut egui::InputState| {
                i.modifiers.is_none() && i.consume_key(egui::Modifiers::NONE, key)
            };
            if ctx.input_mut(bare_key) {
                self.step_selection(step);
            }
        }
    }

    /// Left panel: the machine list. `ui.set_min_width` keeps the
    /// `SidePanel`'s divider draggable even when the list is empty.
    pub(super) fn draw_machine_list(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        self.draw_sort_controls(ui);
        if !self.entries.is_empty() {
            let row_height = machine_row_height(ui);
            let row_spacing = ui.spacing().item_spacing.y;
            let entry_count = self.entries.len();
            let ctx = ui.ctx().clone();
            let scroll_area = egui::ScrollArea::vertical();
            #[cfg(feature = "perf")]
            let scroll_area =
                match self.perf_manager_scroll_target() {
                    Some(target_row) => scroll_area.vertical_scroll_offset(
                        machine_row_scroll_offset(target_row, row_height, row_spacing),
                    ),
                    None => scroll_area,
                };
            #[cfg(feature = "perf")]
            let draw_started = std::time::Instant::now();
            #[cfg(feature = "perf")]
            let mut actual_visible_rows = None;
            scroll_area.show_rows(ui, row_height, entry_count, |ui, visible_rows| {
                #[cfg(feature = "perf")]
                {
                    actual_visible_rows = Some(visible_rows.clone());
                }
                let near_rows = thumbnail_near_range(visible_rows.clone(), entry_count);
                self.prepare_row_thumbnails(&ctx, near_rows, visible_rows.clone());
                ui.set_min_width(ui.available_width());
                if let Some(row) = self.scroll_to_row.take() {
                    let stride = machine_row_stride(row_height, row_spacing);
                    scroll_row_into_view(ui, row, visible_rows.start, row_height, stride);
                }
                for index in visible_rows {
                    self.draw_machine_row(ui, index);
                }
            });
            #[cfg(feature = "perf")]
            if let Some(actual_visible_rows) = actual_visible_rows {
                self.complete_perf_manager_scroll(actual_visible_rows, draw_started.elapsed());
            }
        }
        self.deselect_on_empty_click(ui);
    }

    fn draw_sort_controls(&mut self, ui: &mut egui::Ui) {
        let mut order = self.manager_sort;
        let mut key = order.key();
        ui.add_space(SORT_CONTROLS_TOP_INSET);
        ui.horizontal(|ui| {
            ui.label("Sort by");
            egui::ComboBox::from_id_salt("manager_sort_key")
                .selected_text(key.label())
                .show_ui(ui, |ui| {
                    for candidate in SortKey::ALL {
                        ui.selectable_value(&mut key, candidate, candidate.label());
                    }
                });
            order = order.with_key(key);
            if sort_direction_button(ui, order.is_ascending()).clicked() {
                order = order.toggled();
            }
        });
        if order != self.manager_sort {
            self.change_manager_sort(order);
        }
        if let Some(error) = &self.sort_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.separator();
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
            .show(ui, |ui| self.draw_machine_row_content(ui, i))
            .response
            .rect;

        let click_id = ui.id().with(("machine_row", i));
        let response = ui.interact(frame_rect, click_id, egui::Sense::click());
        if response.clicked() {
            self.apply_row_click(ui, i);
        }
        self.draw_row_context_menu(response, i);
    }

    /// Thumbnail and text columns inside one row's selection frame.
    fn draw_machine_row_content(&self, ui: &mut egui::Ui, i: usize) {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let content_height = row_content_height(ui);
            let entry = &self.entries[i];
            let (texture, uv) = thumbnails::preview_source(entry);
            thumbnails::draw_preview(
                ui,
                egui::vec2(content_height * THUMBNAIL_ASPECT, content_height),
                texture,
                uv,
            );

            let def = &entry.def;
            let config = def
                .to_machine_config()
                .expect("list entries are validated on load/save");
            ui.vertical(|ui| {
                ui.add(egui::Label::new(egui::RichText::new(&def.name).strong()).truncate());
                ui.add(
                    egui::Label::new(format!(
                        "{} · {}",
                        crate::machine_label(config.variant),
                        new_vm::ram_label(config.memory),
                    ))
                    .truncate(),
                );
                ui.add(
                    egui::Label::new(egui::RichText::new(vm_status_label(entry)).weak()).truncate(),
                );
            });
        });
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

    /// ⌘A/Ctrl+A: select every row.
    fn select_all_rows(&mut self) {
        self.selection.select_all(self.entries.len());
        self.on_selection_changed();
    }

    /// ↑/↓: move the selection one row and scroll it into view.
    fn step_selection(&mut self, step: Step) {
        let Some(row) = self.selection.step(step, self.entries.len()) else {
            return;
        };
        self.scroll_to_row = Some(row);
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

fn sort_direction_button(ui: &mut egui::Ui, ascending: bool) -> egui::Response {
    let action = if ascending {
        "Sort descending"
    } else {
        "Sort ascending"
    };
    let side = ui.spacing().interact_size.y;
    let response = ui.add_sized(egui::Vec2::splat(side), egui::Button::new(""));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), action)
    });
    paint_sort_direction_arrow(ui, &response, ascending);
    response.on_hover_text(action)
}

fn paint_sort_direction_arrow(ui: &egui::Ui, response: &egui::Response, ascending: bool) {
    let direction = if ascending { -1.0 } else { 1.0 };
    let center = response.rect.center();
    let tip = center + egui::vec2(0.0, direction * SORT_ARROW_HALF_LENGTH);
    let tail = center - egui::vec2(0.0, direction * SORT_ARROW_HALF_LENGTH);
    let head_y = tip.y - direction * SORT_ARROW_HEAD_LENGTH;
    let stroke = egui::Stroke::new(
        SORT_ARROW_STROKE_WIDTH,
        ui.style().interact(response).text_color(),
    );
    ui.painter().line_segment([tail, tip], stroke);
    ui.painter().line_segment(
        [tip, egui::pos2(tip.x - SORT_ARROW_HEAD_LENGTH, head_y)],
        stroke,
    );
    ui.painter().line_segment(
        [tip, egui::pos2(tip.x + SORT_ARROW_HEAD_LENGTH, head_y)],
        stroke,
    );
}

/// The machine list has the keyboard when nothing else claims it: no
/// focused widget, no modal dialog, no open popup or context menu.
fn list_has_keyboard(ctx: &egui::Context) -> bool {
    !ctx.wants_keyboard_input()
        && ctx.memory(|memory| memory.top_modal_layer().is_none())
        && !egui::Popup::is_any_open(ctx)
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

/// Fixed row height required by [`egui::ScrollArea::show_rows`]. Labels are
/// truncated to one line, so row construction and scrolling use the same
/// geometry even for long machine names.
fn machine_row_height(ui: &egui::Ui) -> f32 {
    row_content_height(ui) + ROW_MARGIN * 2.0
}

fn machine_row_stride(row_height: f32, row_spacing: f32) -> f32 {
    row_height + row_spacing
}

#[cfg(any(feature = "perf", test))]
fn machine_row_scroll_offset(target_row: usize, row_height: f32, row_spacing: f32) -> f32 {
    target_row as f32 * machine_row_stride(row_height, row_spacing)
}

/// Top edge of `row`, measured from the first constructed row's top; `row`
/// may sit on either side of it.
fn machine_row_top(row: usize, first_visible: usize, first_visible_top: f32, stride: f32) -> f32 {
    first_visible_top + (row as f32 - first_visible as f32) * stride
}

/// Inside `show_rows`' closure, whose `ui` starts at `first_visible`'s top:
/// scroll just far enough to show `row`, constructed or not.
fn scroll_row_into_view(
    ui: &egui::Ui,
    row: usize,
    first_visible: usize,
    row_height: f32,
    stride: f32,
) {
    let top = machine_row_top(row, first_visible, ui.max_rect().top(), stride);
    let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=top + row_height);
    ui.scroll_to_rect(rect, None);
}

/// Expands the constructed row range into the preview-preload range without
/// exceeding the machine library.
fn thumbnail_near_range(
    visible_rows: std::ops::Range<usize>,
    entry_count: usize,
) -> std::ops::Range<usize> {
    visible_rows.start.saturating_sub(THUMBNAIL_NEAR_RANGE_ROWS)
        ..visible_rows
            .end
            .saturating_add(THUMBNAIL_NEAR_RANGE_ROWS)
            .min(entry_count)
}

#[cfg(test)]
#[path = "list_test.rs"]
mod tests;
