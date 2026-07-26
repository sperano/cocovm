//! Disassembly panel: a scrolling, breakpoint-gutter-annotated live
//! disassembly, following PC or a manually navigated address.

use coco_core::Machine;
use eframe::egui;
use mc6809::disasm::disassemble;

use super::DebuggerPanel;

/// Disassembly rows shown per frame.
const DISASM_ROWS: usize = 32;

impl DebuggerPanel {
    pub(super) fn disasm_ui(&mut self, ui: &mut egui::Ui, machine: &Machine) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.disasm_follow_pc, "Follow PC");
            ui.add(egui::TextEdit::singleline(&mut self.disasm_goto_text).desired_width(60.0));
            if ui.button("Go").clicked()
                && let Some(addr) = super::parse_addr(&self.disasm_goto_text)
            {
                self.disasm_addr = addr;
                self.disasm_follow_pc = false;
            }
        });
        if self.disasm_follow_pc {
            self.disasm_addr = machine.cpu.pc;
        }
        egui::ScrollArea::vertical()
            .id_salt("dbg_disasm_scroll")
            .max_height(420.0)
            .show(ui, |ui| {
                let mut addr = self.disasm_addr;
                for _ in 0..DISASM_ROWS {
                    let insn = disassemble(&mut |a| machine.bus.peek(a), addr);
                    let row_addr = addr;
                    ui.horizontal(|ui| {
                        let mut has_bp = self
                            .core
                            .breakpoint(row_addr)
                            .is_some_and(|bp| bp.enabled);
                        if ui.checkbox(&mut has_bp, "").changed() {
                            if has_bp {
                                self.core.add_breakpoint(row_addr);
                            } else {
                                self.core.remove_breakpoint(row_addr);
                            }
                        }
                        let is_pc = row_addr == machine.cpu.pc;
                        let text = format!(
                            "{}${row_addr:04X}  {:<6}{}",
                            if is_pc { "> " } else { "  " },
                            insn.mnemonic,
                            insn.operand
                        );
                        let mut rich = egui::RichText::new(text);
                        if is_pc {
                            rich = rich.strong().color(egui::Color32::YELLOW);
                        }
                        let selected = self.cursor_addr == Some(row_addr);
                        if ui.selectable_label(selected, rich).clicked() {
                            self.cursor_addr = Some(row_addr);
                        }
                    });
                    addr = addr.wrapping_add(u16::from(insn.len.max(1)));
                }
            });
    }
}
