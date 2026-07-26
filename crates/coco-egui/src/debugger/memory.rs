//! Memory panel: the Logical/Physical toggle, the hex+ASCII grid (with
//! watchpoint highlighting), and the watchpoint list underneath it.

use coco_core::config::BLOCK_SIZE;
use coco_core::Machine;
use eframe::egui;

use super::{parse_addr, parse_addr_u32, ascii_char, DebuggerPanel, MemoryView};

/// Memory grid dimensions (bytes per row × rows shown per frame).
const MEM_COLS: usize = 16;
const MEM_ROWS: usize = 16;

impl DebuggerPanel {
    pub(super) fn memory_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, running: bool) {
        let editable = !running;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mem_view, MemoryView::Logical, "Logical (MMU)");
            ui.selectable_value(&mut self.mem_view, MemoryView::Physical, "Physical RAM");
        });
        ui.horizontal(|ui| {
            if self.mem_view == MemoryView::Physical {
                let max_block = (machine.bus.ram.len() / BLOCK_SIZE).saturating_sub(1);
                let mut block = (self.mem_phys_addr as usize / BLOCK_SIZE).min(max_block);
                ui.label("Block:");
                if ui
                    .add(egui::DragValue::new(&mut block).range(0..=max_block))
                    .changed()
                {
                    self.mem_phys_addr = (block * BLOCK_SIZE) as u32;
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.mem_goto_text).desired_width(70.0));
            if ui.button("Go").clicked() {
                match self.mem_view {
                    MemoryView::Logical => {
                        if let Some(addr) = parse_addr(&self.mem_goto_text) {
                            self.mem_addr = addr;
                        }
                    }
                    MemoryView::Physical => {
                        if let Some(addr) = parse_addr_u32(&self.mem_goto_text) {
                            self.mem_phys_addr = addr;
                        }
                    }
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("dbg_mem_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                egui::Grid::new("dbg_mem_grid").striped(true).show(ui, |ui| {
                    for row in 0..MEM_ROWS {
                        self.memory_row_ui(ui, machine, row, editable);
                        ui.end_row();
                    }
                });
            });
        ui.separator();
        self.watchpoints_ui(ui);
    }

    /// One row of the Memory grid: the row's base address, `MEM_COLS` editable
    /// hex byte cells, then the ASCII rendering. Split out of
    /// [`Self::memory_ui`] purely to keep that function's nesting readable.
    fn memory_row_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, row: usize, editable: bool) {
        let logical = self.mem_view == MemoryView::Logical;
        let row_addr_logical = self.mem_addr.wrapping_add((row * MEM_COLS) as u16);
        let ram_len = machine.bus.ram.len().max(1);
        let row_base_phys = (self.mem_phys_addr as usize + row * MEM_COLS) % ram_len;

        ui.label(if logical {
            format!("${row_addr_logical:04X}")
        } else {
            format!("{row_base_phys:06X}")
        });

        let mut ascii = String::with_capacity(MEM_COLS);
        for col in 0..MEM_COLS {
            let (mut val, watched) = if logical {
                let addr = row_addr_logical.wrapping_add(col as u16);
                (
                    machine.bus.peek(addr),
                    self.core.watchpoint(addr).is_some_and(|wp| wp.enabled),
                )
            } else {
                let idx = (row_base_phys + col) % ram_len;
                (machine.bus.ram[idx], false)
            };
            let before = val;
            let drag = egui::DragValue::new(&mut val).hexadecimal(2, false, true);
            let resp = if watched {
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(90, 60, 0))
                    .show(ui, |ui| ui.add_enabled(editable, drag))
                    .inner
            } else {
                ui.add_enabled(editable, drag)
            };
            if resp.changed() && val != before {
                if logical {
                    let addr = row_addr_logical.wrapping_add(col as u16);
                    machine.poke(addr, val);
                } else {
                    let idx = (row_base_phys + col) % ram_len;
                    machine.bus.ram[idx] = val;
                }
            }
            ascii.push(ascii_char(val));
        }
        ui.label(ascii);
    }

    fn watchpoints_ui(&mut self, ui: &mut egui::Ui) {
        ui.label("Watchpoints (logical address)");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.watch_goto_text).desired_width(60.0));
            ui.checkbox(&mut self.watch_read, "R");
            ui.checkbox(&mut self.watch_write, "W");
            if ui.button("Add").clicked()
                && let Some(addr) = parse_addr(&self.watch_goto_text)
            {
                self.core.add_watchpoint(addr, self.watch_read, self.watch_write);
            }
        });
        let watches: Vec<(u16, bool, bool, bool, u64)> = self
            .core
            .watchpoints()
            .map(|(addr, wp)| (addr, wp.read, wp.write, wp.enabled, wp.hits))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("dbg_watch_list")
            .max_height(80.0)
            .show(ui, |ui| {
                for (addr, read, write, mut enabled, hits) in watches {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut enabled, "").changed() {
                            self.core.set_watchpoint_enabled(addr, enabled);
                        }
                        let dir = match (read, write) {
                            (true, true) => "RW",
                            (true, false) => "R",
                            (false, true) => "W",
                            (false, false) => "-",
                        };
                        ui.label(format!("${addr:04X} {dir} hits:{hits}"));
                        if ui.button("x").clicked() {
                            self.core.remove_watchpoint(addr);
                        }
                    });
                }
            });
    }
}
