//! Registers panel: the CPU register grid (A/B/D/X/Y/U/S/PC/DP), the CC
//! flag checkboxes, and the cycle/scanline readout.

use coco_core::Machine;
use eframe::egui;
use mc6809::cc;

use super::DebuggerPanel;

impl DebuggerPanel {
    pub(super) fn registers_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, running: bool) {
        let editable = !running;
        egui::Grid::new("dbg_regs_grid")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("A");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.a).hexadecimal(2, false, true),
                );
                ui.end_row();
                ui.label("B");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.b).hexadecimal(2, false, true),
                );
                ui.end_row();
                ui.label("D");
                ui.label(format!(
                    "${:04X}",
                    (u16::from(machine.cpu.a) << 8) | u16::from(machine.cpu.b)
                ));
                ui.end_row();
                ui.label("X");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.x).hexadecimal(4, false, true),
                );
                ui.end_row();
                ui.label("Y");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.y).hexadecimal(4, false, true),
                );
                ui.end_row();
                ui.label("U");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.u).hexadecimal(4, false, true),
                );
                ui.end_row();
                ui.label("S");
                let mut s_reg = machine.cpu.s;
                if ui
                    .add_enabled(
                        editable,
                        egui::DragValue::new(&mut s_reg).hexadecimal(4, false, true),
                    )
                    .changed()
                {
                    // Route through load_s, not a direct write, so NMI arming is preserved.
                    machine.cpu.load_s(s_reg);
                }
                ui.end_row();
                ui.label("PC");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.pc).hexadecimal(4, false, true),
                );
                ui.end_row();
                ui.label("DP");
                ui.add_enabled(
                    editable,
                    egui::DragValue::new(&mut machine.cpu.dp).hexadecimal(2, false, true),
                );
                ui.end_row();
            });
        ui.separator();
        ui.horizontal(|ui| {
            // EFHINZVC, matching the CC bit order top (E, $80) to bottom (C, $01).
            for (label, bit) in [
                ("E", cc::ENTIRE),
                ("F", cc::FIRQ_MASK),
                ("H", cc::HALF_CARRY),
                ("I", cc::IRQ_MASK),
                ("N", cc::NEGATIVE),
                ("Z", cc::ZERO),
                ("V", cc::OVERFLOW),
                ("C", cc::CARRY),
            ] {
                let mut flag = machine.cpu.cc & bit != 0;
                if ui
                    .add_enabled(editable, egui::Checkbox::new(&mut flag, label))
                    .changed()
                {
                    if flag {
                        machine.cpu.cc |= bit;
                    } else {
                        machine.cpu.cc &= !bit;
                    }
                }
            }
        });
        ui.separator();
        ui.label(format!("cycles: {}", machine.cpu.cycles));
        ui.label(format!("scanline: {}", machine.current_scanline()));
    }
}
