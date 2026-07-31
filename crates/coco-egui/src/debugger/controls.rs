//! Controls panel: Run/Pause, the four step commands, the cursor readout,
//! and the breakpoint list.

use coco_core::Machine;
use eframe::egui;

use super::{DebuggerPanel, parse_addr};

impl DebuggerPanel {
    pub(super) fn controls_ui(
        &mut self,
        ui: &mut egui::Ui,
        machine: &mut Machine,
        running: &mut bool,
    ) {
        ui.horizontal(|ui| {
            let run_label = if *running { "Pause" } else { "Run" };
            if ui.button(run_label).clicked() {
                *running = !*running;
            }
            ui.add_enabled_ui(!*running, |ui| {
                if ui.button("Step In").clicked() {
                    Self::step_in(machine);
                }
                if ui.button("Step Over").clicked() {
                    self.step_over(machine);
                }
                if ui.button("Step Out").clicked() {
                    Self::step_out(machine);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!*running, |ui| {
                if ui.button("Step Scanline").clicked() {
                    Self::step_scanline(machine);
                }
                if ui.button("Step Field").clicked() {
                    machine.run_field();
                }
                let target = self.cursor_addr;
                if ui
                    .add_enabled(target.is_some(), egui::Button::new("Run to Cursor"))
                    .clicked()
                    && let Some(addr) = target
                {
                    self.run_to(machine, addr);
                }
            });
        });
        ui.label(match self.cursor_addr {
            Some(addr) => format!("cursor: ${addr:04X} (click a disassembly row to change)"),
            None => "cursor: none (click a disassembly row)".to_string(),
        });

        ui.separator();
        ui.label("Breakpoints");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.bp_goto_text).desired_width(60.0));
            if ui.button("Add").clicked()
                && let Some(addr) = parse_addr(&self.bp_goto_text)
            {
                self.core.add_breakpoint(addr);
            }
        });
        let breakpoints: Vec<(u16, bool, u64)> = self
            .core
            .breakpoints()
            .map(|(addr, bp)| (addr, bp.enabled, bp.hits))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("dbg_bp_list")
            .max_height(100.0)
            .show(ui, |ui| {
                for (addr, mut enabled, hits) in breakpoints {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut enabled, "").changed() {
                            self.core.set_breakpoint_enabled(addr, enabled);
                        }
                        ui.label(format!("${addr:04X}  hits:{hits}"));
                        if ui.button("x").clicked() {
                            self.core.remove_breakpoint(addr);
                        }
                    });
                }
            });
    }
}
