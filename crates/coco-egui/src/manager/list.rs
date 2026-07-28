//! The machine-list panel: [`ManagerApp::draw_machine_list`] and one row's
//! worth of drawing/interaction ([`ManagerApp::draw_machine_row`]), plus the
//! row-thumbnail rendering helpers only this panel needs.

use eframe::egui;

use crate::new_vm;
use super::{
    vm_status_label, CocoApp, ManagerApp, ROW_CORNER_RADIUS, ROW_MARGIN, SUSPEND_DISABLED_HOVER,
    THUMBNAIL_ASPECT, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_PLACEHOLDER_FILL,
};

impl ManagerApp {
    /// Left panel: the machine list. `ui.set_min_width` (rather than only
    /// `take_available_space` on the empty case) keeps the `SidePanel`'s
    /// divider draggable in both states — an empty ui claims no space,
    /// which disables the resize drag (`SidePanel::resizable` docs).
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
    /// selection, bringing the photo pane back (the manager's "click the
    /// desktop to deselect" gesture). The edit state is dropped too, so the
    /// next selection reseeds fresh — same as switching rows. Sensing only
    /// clicks leaves the `SidePanel` divider's *drag* untouched even where
    /// the two regions overlap (egui resolves click and drag hits per
    /// sense — `manager_list_divider_is_draggable` guards this).
    fn deselect_on_empty_click(&mut self, ui: &mut egui::Ui) {
        let remaining = ui.available_size_before_wrap();
        if remaining.y <= 0.0 {
            return;
        }
        let (_, response) = ui.allocate_exact_size(remaining, egui::Sense::click());
        if response.clicked() && self.selected.is_some() {
            self.selected = None;
            self.edit = None;
            self.save_error = None;
        }
    }

    /// One machine-list row: placeholder thumbnail + name/subtitle/status
    /// for an `Ok` entry, or the file stem + an error badge for an `Err`
    /// one. Clicking anywhere in the row selects it (`ui.interact` over the
    /// frame's rect — the row's own labels aren't themselves interactive).
    pub(super) fn draw_machine_row(&mut self, ui: &mut egui::Ui, i: usize) {
        // A stopped machine's saved preview, if any, loads (once) before the
        // row draws so this frame can already show it.
        self.ensure_row_thumbnail(&ui.ctx().clone(), i);
        let selected = self.selected == Some(i);
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
                    // Preview priority: a live VM's framebuffer texture,
                    // else the saved thumbnail.png loaded above, else the
                    // bare placeholder fill.
                    let texture = self.entries[i]
                        .vm
                        .as_deref()
                        .and_then(CocoApp::framebuffer_texture)
                        .or(self.entries[i].thumbnail.as_ref());
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
            self.selected = Some(i);
            self.save_error = None;
        }
        self.draw_row_context_menu(response, i);
    }

    /// Per-row context menu. Its items act on the row under the cursor
    /// (this `i`), never on `self.selected` — right-click deliberately
    /// does not move the selection cue (user decision 2026-07-23); only
    /// "Show config" moves it, because showing the detail pane *is*
    /// selecting.
    fn draw_row_context_menu(&mut self, response: egui::Response, i: usize) {
        response.context_menu(|ui| {
            let has_vm = self.entries[i].vm.is_some();
            if ui.add_enabled(!has_vm, egui::Button::new("Start")).clicked() {
                self.start_vm(i);
                ui.close();
            }
            let _ = ui
                .add_enabled(false, egui::Button::new("Suspend"))
                .on_disabled_hover_text(SUSPEND_DISABLED_HOVER);
            if ui.add_enabled(has_vm, egui::Button::new("Reset")).clicked() {
                if let Some(vm) = self.entries[i].vm.as_mut() {
                    vm.machine.reset();
                }
                ui.close();
            }
            if ui.add_enabled(has_vm, egui::Button::new("Stop")).clicked() {
                self.stop_vm(i);
                ui.close();
            }
            ui.separator();
            if ui.button("Show config").clicked() {
                self.selected = Some(i);
                self.save_error = None;
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                self.pending_delete = Some(self.entries[i].slug.clone());
                ui.close();
            }
        });
    }
}

/// Height the row's own text column (name, subtitle, status — three
/// `TextStyle::Body`-sized lines with `ui.vertical`'s default item spacing
/// between them) will render at, used to size the thumbnail to reach the
/// same bottom edge as the status line (user follow-up to step 6: "should
/// use the height available... go to the same edge as Stopped"). Computed
/// from text metrics up front rather than measured after layout, since the
/// thumbnail is the *first* widget placed in the row's `horizontal` — by
/// the time the text column's actual rendered height is known, the
/// thumbnail's own space is already allocated. All three lines use the
/// default `Body` text style at its default size (`.strong()`/`ui.weak()`
/// only change weight/color, not size), so one line height covers all
/// three, and `ui.vertical`'s gaps are exactly `ui.spacing().item_spacing.y`
/// — reproducing both here needs no second/probing layout pass.
fn row_content_height(ui: &egui::Ui) -> f32 {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
    let spacing = ui.spacing().item_spacing.y;
    line_height * 3.0 + spacing * 2.0
}

/// One list row's thumbnail: the resolved preview `texture` — a live VM's
/// framebuffer, or a stopped machine's saved [`super::THUMBNAIL_FILE`]; the
/// caller resolves that priority — sized to `height` tall (see
/// [`row_content_height`]) at the fixed [`THUMBNAIL_ASPECT`], the same 4:3
/// the emulator's own display corrects to (framebuffer pixels aren't
/// square, so the texture's raw aspect would stretch the picture). It's one
/// extra quad reusing an already-uploaded texture, not an extra upload
/// (`docs/plan-machine-persistence.md` step 6, "Running/paused VM" bullet).
/// A paused VM's texture simply stops changing, so the thumbnail freezes on
/// its last frame with no special casing needed. With no texture — a
/// stopped machine, or a VM whose first frame hasn't uploaded one yet —
/// just the placeholder fill shows. Allocates its own space and returns the
/// rect it claimed.
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
