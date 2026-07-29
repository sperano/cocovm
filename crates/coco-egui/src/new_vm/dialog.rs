//! [`NewVmDialog`]'s impl: opening/closing it and drawing it — the
//! direct-boot "Machine → New…" window built around a shared
//! [`super::MachineForm`].

use coco_core::MachineConfig;
use eframe::egui;

use super::{
    ram_choices, ram_label, MachineForm, NewMachineSpec, NewVmAction, NewVmDialog, DIALOG_MARGIN,
    DIALOG_MIN_SIZE, FORM_GRID_SPACING,
};

impl NewVmDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            error: None,
            form: MachineForm::new("new_vm", false),
        }
    }

    /// Open the dialog with the form seeded from the running machine —
    /// config and UI preferences — so "New…" defaults to "same machine
    /// again", with nothing mounted.
    pub fn open_with(&mut self, current: MachineConfig, aspect_correct: bool, kb_mode: crate::KbMode) {
        self.form.reset_inventory();
        self.form.config = current;
        self.form.aspect_correct = aspect_correct;
        self.form.kb_mode = kb_mode;
        self.error = None;
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.error = None;
    }

    /// Draw the dialog if open. Returns [`NewVmAction::Create`] on the frame
    /// "Create" is clicked; the dialog stays open so a failure can be shown
    /// inline (the caller closes it on success).
    pub fn show(&mut self, ctx: &egui::Context) -> NewVmAction {
        if !self.open {
            return NewVmAction::None;
        }
        let mut action = NewVmAction::None;
        let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
        let mut open = self.open;
        egui::Window::new(crate::window_title(ctx, "New Machine"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .min_size(DIALOG_MIN_SIZE)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // The frame only ever hugs the content (see
                // [`DIALOG_MIN_SIZE`]), so claim the floor — and any extra
                // room from a drag-resize — as the content's own size.
                ui.set_min_size(DIALOG_MIN_SIZE.max(ui.available_size()));
                egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                    egui::Grid::new("new_vm_grid")
                        .num_columns(2)
                        .spacing(FORM_GRID_SPACING)
                        .show(ui, |ui| {
                            self.form.rows(ui);
                        });

                    // DEMO (temporary branch): `widgets::titled_group`, the
                    // fieldset-style container egui lacks, wrapping a second
                    // live RAM picker. It edits the same draft as the grid's
                    // own RAM row above, so the two combos stay in sync —
                    // immediate mode's one-source-of-truth for free.
                    ui.add_space(DIALOG_MARGIN as f32 / 2.0);
                    crate::widgets::titled_group(ui, "RAM", |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::Grid::new("new_vm_ram_demo")
                            .num_columns(2)
                            .spacing(FORM_GRID_SPACING)
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("Size").size(font));
                                egui::ComboBox::from_id_salt(("new_vm_demo", "ram"))
                                    .selected_text(ram_label(self.form.config.memory))
                                    .show_ui(ui, |ui| {
                                        for &memory in ram_choices(self.form.config.variant) {
                                            ui.selectable_value(
                                                &mut self.form.config.memory,
                                                memory,
                                                ram_label(memory),
                                            );
                                        }
                                    });
                                ui.end_row();
                            });
                    });

                    if let Some(error) = &self.error {
                        ui.add_space(DIALOG_MARGIN as f32 / 2.0);
                        ui.label(
                            egui::RichText::new(error)
                                .size(font)
                                .color(ui.visuals().error_fg_color),
                        );
                    }
                    ui.add_space(DIALOG_MARGIN as f32);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                        if ui.button("Create").clicked() {
                            action = NewVmAction::Create(Box::new(NewMachineSpec {
                                config: self.form.config,
                                cartridge: self.form.cartridge.clone(),
                                mpi_slots: self.form.mpi_slots.clone(),
                                disks: self.form.disks.clone(),
                                tape: self.form.tape.clone(),
                                vhds: self.form.vhds.clone(),
                                aspect_correct: self.form.aspect_correct,
                                kb_mode: self.form.kb_mode,
                            }));
                        }
                        if ui.button("Cancel").clicked() {
                            self.close();
                        }
                    });
                });
            });
        // `open` only goes false via the title-bar close box; `self.close()`
        // inside the body must not be resurrected by writing `open` back.
        if !open {
            self.close();
        }
        action
    }
}
