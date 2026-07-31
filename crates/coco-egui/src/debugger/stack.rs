//! Stack panel: S-relative 16-bit slots with a best-effort "this looks like
//! a return address" disassembly hint next to each one.

use coco_core::Machine;
use eframe::egui;
use mc6809::disasm::disassemble;

use super::DebuggerPanel;

/// Stack-relative 16-bit slots shown (S, S+2, S+4, ... up to this many).
const STACK_SLOTS: usize = 16;

impl DebuggerPanel {
    pub(super) fn stack_ui(&self, ui: &mut egui::Ui, machine: &Machine) {
        ui.label(format!("S = ${:04X}", machine.cpu.s));
        egui::ScrollArea::vertical()
            .id_salt("dbg_stack_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                egui::Grid::new("dbg_stack_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        for i in 0..STACK_SLOTS {
                            let addr = machine.cpu.s.wrapping_add((i * 2) as u16);
                            let hi = machine.bus.peek(addr);
                            let lo = machine.bus.peek(addr.wrapping_add(1));
                            let word = (u16::from(hi) << 8) | u16::from(lo);
                            // Candidate return-address annotation: disassemble
                            // whatever is AT this 16-bit stack value, so a
                            // genuine return address reads as recognizable code
                            // next to it (`docs/plan-debugger.md` §3) — not every
                            // slot holds one (locals, saved registers), so this
                            // is a best-effort hint, not a claim.
                            let insn = disassemble(&mut |a| machine.bus.peek(a), word);
                            ui.label(format!("${addr:04X}"));
                            ui.label(format!("${word:04X}"));
                            ui.label(format!("-> {} {}", insn.mnemonic, insn.operand));
                            ui.end_row();
                        }
                    });
            });
    }
}
