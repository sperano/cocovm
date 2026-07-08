//! Interactive debugger UI (`docs/plan-debugger.md` §3): Controls, Registers,
//! Disassembly, Memory, Stack, and Hardware-state panels, toggled with F11
//! (see `main.rs`'s `handle_input` — F9/F10/F12 are already taken).
//!
//! Shown as its own native OS window (an egui *immediate viewport*, like the
//! printer's `paper_view`), so the panels never cover the emulated screen;
//! backends without multi-window support fall back to floating panels in the
//! main viewport.
//!
//! [`DebuggerPanel`] owns the `coco_core::debug::Debugger` (breakpoints,
//! watchpoints, trace ring) and is the single entry point `CocoApp::update`
//! drives the per-field run loop through ([`DebuggerPanel::run_field`]) so a
//! tripped breakpoint or watchpoint pauses the emulator the same way the
//! Run/Pause button does, rather than needing a second "why did we stop"
//! flag. Every read view (disassembly, memory, stack) goes through
//! [`coco_core::SystemBus::peek`] — never `read` — so simply having the
//! debugger open can never perturb PIA/GIME/cart state (`docs/plan-debugger.md`
//! §2, "side-effect-free reads").
//!
//! Widgets are editable only while paused (`ui.add_enabled(!running, ..)`);
//! while running the panels still redraw every frame from live `peek`s, same
//! as the main screen.

use coco_core::config::BLOCK_SIZE;
use coco_core::debug::Debugger;
use coco_core::debug::StopReason;
use coco_core::gime::{self, init0, init1};
use coco_core::pia::{cr, MC6821};
use coco_core::{Machine, StepKind};
use eframe::egui;
use mc6809::cc;
use mc6809::disasm::disassemble;

/// Instructions handed to one [`Debugger::run_until`] call before this module
/// re-checks its own bookkeeping. Comfortably above one field's instruction
/// count (a few thousand at most, even at double CPU speed), so a plain run
/// with nothing tripped always returns via `StopReason::FieldComplete` well
/// inside this budget — [`StopReason::Step`] (budget exhausted) is the rare,
/// handled fallback, not the common case.
const RUN_BUDGET: u64 = 200_000;

/// Safety cap on `run_until` calls chained across `FieldComplete`/`Step` while
/// hunting for a temporary breakpoint (Step Over, Run-to-Cursor) or driving a
/// plain "Run": bounds how long an unattended run can spin — a callee that
/// never returns, or a cursor address that's never reached — before this
/// gives up, so it can't hang the UI thread forever. ~1000 fields is minutes
/// of emulated time, generous for any real call/run.
const MAX_CHAINED_RUNS: u32 = 1_000;

/// Safety cap on raw instruction-by-instruction stepping (Step In / Step Out /
/// Step Scanline), which step the CPU directly rather than through
/// `Debugger::run_until` and so have no field-boundary checkpoint to chain
/// from.
const MAX_RAW_STEPS: u32 = 2_000_000;

/// Disassembly rows shown per frame.
const DISASM_ROWS: usize = 32;
/// Memory grid dimensions (bytes per row × rows shown per frame).
const MEM_COLS: usize = 16;
const MEM_ROWS: usize = 16;
/// Stack-relative 16-bit slots shown (S, S+2, S+4, ... up to this many).
const STACK_SLOTS: usize = 16;

/// Which address space the Memory panel is viewing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MemoryView {
    /// Through the live MMU, exactly as the running CPU sees it (`peek`).
    Logical,
    /// Raw installed RAM, bypassing the MMU — a bank/block picker selects
    /// where in the (up to 2048K) physical space the view starts.
    Physical,
}

/// Panel toggle plus every panel's own navigation/edit state, and the
/// [`Debugger`] core it drives. One instance lives in `CocoApp`.
pub struct DebuggerPanel {
    /// Master F11 toggle — when false, `windows_ui` draws nothing.
    pub open: bool,
    core: Debugger,

    // ---- Disassembly panel --------------------------------------------
    disasm_follow_pc: bool,
    /// Top-of-view address when `disasm_follow_pc` is off.
    disasm_addr: u16,
    disasm_goto_text: String,
    /// Row last clicked (not the breakpoint gutter) — the "Run to Cursor"
    /// target.
    cursor_addr: Option<u16>,

    // ---- Memory panel ---------------------------------------------------
    mem_view: MemoryView,
    /// Top-of-view address for [`MemoryView::Logical`].
    mem_addr: u16,
    /// Top-of-view byte offset for [`MemoryView::Physical`] (up to the
    /// installed RAM size, so wider than a CPU logical address).
    mem_phys_addr: u32,
    mem_goto_text: String,
    watch_goto_text: String,
    watch_read: bool,
    watch_write: bool,

    // ---- Controls panel --------------------------------------------------
    bp_goto_text: String,
}

impl Default for DebuggerPanel {
    fn default() -> Self {
        Self {
            open: false,
            core: Debugger::new(),
            disasm_follow_pc: true,
            disasm_addr: 0,
            disasm_goto_text: String::new(),
            cursor_addr: None,
            mem_view: MemoryView::Logical,
            mem_addr: 0,
            mem_phys_addr: 0,
            mem_goto_text: String::new(),
            watch_goto_text: String::new(),
            watch_read: true,
            watch_write: true,
            bp_goto_text: String::new(),
        }
    }
}

impl DebuggerPanel {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- Run-loop integration ----------------------------------------------

    /// Run one video field through the debugger core, for `CocoApp::update`'s
    /// running loop to call instead of `Machine::run_field` directly. Chains
    /// `run_until` calls across `FieldComplete`/`Step` until the field
    /// actually completes (matching `Machine::run_field`'s contract exactly
    /// when no breakpoint/watchpoint is set — the common case) or a
    /// breakpoint/watchpoint trips, in which case this returns `false` so the
    /// caller can pause (`self.running = false`), same as clicking Pause.
    pub fn run_field(&mut self, machine: &mut Machine) -> bool {
        for _ in 0..MAX_CHAINED_RUNS {
            match self.core.run_until(machine, RUN_BUDGET) {
                // The field this call was asked to run is done — stop here,
                // unlike `run_until_stop`, which wants to keep going past
                // field boundaries. Conflating the two was a real bug caught
                // by `run_field_with_no_breakpoints_matches_plain_run_field`:
                // treating `FieldComplete` as "keep looping" silently ran
                // extra fields per call.
                StopReason::FieldComplete => return true,
                // Budget exhausted before either a field boundary or a stop
                // condition — keep going within the same field.
                StopReason::Step => continue,
                StopReason::Breakpoint(_) | StopReason::Watchpoint { .. } => return false,
            }
        }
        // A single field needing more than MAX_CHAINED_RUNS * RUN_BUDGET
        // instructions isn't physically possible at real CoCo timings; report
        // "continue" rather than spuriously acting like a breakpoint fired.
        true
    }

    /// Chain `run_until` calls the same way [`Self::run_field`] does, but
    /// without a field-boundary exit condition — used by Step Over/Run-to-
    /// Cursor, which want to run past any number of field boundaries until a
    /// breakpoint (their own temporary one, or any other enabled one) or
    /// watchpoint trips, or the safety cap gives up.
    fn run_until_stop(&mut self, machine: &mut Machine) {
        for _ in 0..MAX_CHAINED_RUNS {
            match self.core.run_until(machine, RUN_BUDGET) {
                StopReason::FieldComplete | StopReason::Step => continue,
                StopReason::Breakpoint(_) | StopReason::Watchpoint { .. } => return,
            }
        }
    }

    /// Run until `target` (a temporary breakpoint, unless one already exists
    /// there — in which case the user's own breakpoint is left alone
    /// afterwards) or any other stop condition trips first.
    fn run_to(&mut self, machine: &mut Machine, target: u16) {
        let had_existing = self.core.breakpoint(target).is_some();
        if !had_existing {
            self.core.add_breakpoint(target);
        }
        self.run_until_stop(machine);
        if !had_existing {
            self.core.remove_breakpoint(target);
        }
    }

    // ---- Stepping primitives (bypass the Debugger core — see each doc) ----

    /// Step In: exactly one retired instruction, skipping over (not stopping
    /// on) any burned HALT* cycles — a click should always advance real CPU
    /// state, not just burn one HALT cycle. Does not consult breakpoints:
    /// it's a single deterministic step already fully under the user's
    /// control.
    fn step_in(machine: &mut Machine) {
        for _ in 0..MAX_RAW_STEPS {
            if matches!(machine.step_instruction().kind, StepKind::Instruction { .. }) {
                return;
            }
        }
    }

    /// Step Over: temp-breakpoints past a call instruction (JSR/BSR/LBSR) so
    /// the callee runs to completion in one step; falls back to Step In for
    /// every other opcode (`docs/plan-debugger.md` §3).
    fn step_over(&mut self, machine: &mut Machine) {
        let pc = machine.cpu.pc;
        let insn = disassemble(&mut |a| machine.bus.peek(a), pc);
        if !matches!(insn.mnemonic, "JSR" | "BSR" | "LBSR") {
            Self::step_in(machine);
            return;
        }
        let return_addr = pc.wrapping_add(u16::from(insn.len));
        self.run_to(machine, return_addr);
    }

    /// Step Out: run until S rises past its value at the start of the call
    /// (`docs/plan-debugger.md` §3) — i.e. until the enclosing subroutine's
    /// RTS has popped the return address. Checked after every real
    /// instruction, so (unlike Step Over/Run-to-Cursor) this does not chain
    /// through `Debugger::run_until` and consequently does not stop early for
    /// a breakpoint/watchpoint hit inside the callee — a known limitation,
    /// acceptable because the S-rise condition itself isn't something the
    /// Debugger core's breakpoint/watchpoint machinery can express.
    fn step_out(machine: &mut Machine) {
        let s0 = machine.cpu.s;
        for _ in 0..MAX_RAW_STEPS {
            let ev = machine.step_instruction();
            if matches!(ev.kind, StepKind::Instruction { .. }) && machine.cpu.s > s0 {
                return;
            }
        }
    }

    /// Step Scanline: run until the current scanline counter changes (a
    /// per-line trailer ran) or the field wraps.
    fn step_scanline(machine: &mut Machine) {
        let start = machine.current_scanline();
        for _ in 0..MAX_RAW_STEPS {
            let ev = machine.step_instruction();
            if ev.field_complete || machine.current_scanline() != start {
                return;
            }
        }
    }

    // ---- Top-level UI -------------------------------------------------------

    /// Draw every panel, if [`Self::open`]. Called unconditionally once per
    /// `update()`, like the app's other optional windows.
    ///
    /// The panels live in their own native OS window (an egui *immediate
    /// viewport*, same pattern as `paper_view::PaperWindow::ui`) so the
    /// debugger never covers the emulated screen. On a backend without
    /// native multi-window support egui reports `ViewportClass::Embedded`
    /// and the panels fall back to floating over the main viewport.
    pub fn windows_ui(&mut self, ctx: &egui::Context, machine: &mut Machine, running: &mut bool) {
        if !self.open {
            return;
        }
        // One stable ID so egui reuses the same native OS window across
        // frames instead of spawning a new one.
        let viewport_id = egui::ViewportId::from_hash_of("debugger");
        let builder = egui::ViewportBuilder::default()
            .with_title("Debugger")
            .with_inner_size([1140.0, 780.0])
            .with_min_inner_size([480.0, 320.0]);
        ctx.show_viewport_immediate(viewport_id, builder, |ctx, class| {
            if class != egui::ViewportClass::Embedded {
                // Backdrop for the panel cluster to float over; without it
                // the viewport is unpainted.
                egui::CentralPanel::default().show(ctx, |_ui| {});
                // The OS close button: accept the close by not showing the
                // viewport next frame (mirrors the F11 / View-menu toggle).
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.open = false;
                }
            }
            self.panel_windows(ctx, machine, running);
        });
    }

    /// The six panel windows. Inside [`Self::windows_ui`]'s viewport closure
    /// they render into the debugger's native window; on the embedded
    /// fallback path they land in the main viewport, exactly as before the
    /// debugger became a native window.
    fn panel_windows(&mut self, ctx: &egui::Context, machine: &mut Machine, running: &mut bool) {
        egui::Window::new(crate::window_title(ctx, "Debug: Controls"))
            .default_pos([20.0, 40.0])
            .show(ctx, |ui| self.controls_ui(ui, machine, running));
        egui::Window::new(crate::window_title(ctx, "Debug: Registers"))
            .default_pos([20.0, 230.0])
            .show(ctx, |ui| self.registers_ui(ui, machine, *running));
        egui::Window::new(crate::window_title(ctx, "Debug: Disassembly"))
            .default_pos([320.0, 40.0])
            .default_width(340.0)
            .show(ctx, |ui| self.disasm_ui(ui, machine));
        egui::Window::new(crate::window_title(ctx, "Debug: Memory"))
            .default_pos([680.0, 40.0])
            .default_width(460.0)
            .show(ctx, |ui| self.memory_ui(ui, machine, *running));
        egui::Window::new(crate::window_title(ctx, "Debug: Stack"))
            .default_pos([320.0, 420.0])
            .show(ctx, |ui| self.stack_ui(ui, machine));
        egui::Window::new(crate::window_title(ctx, "Debug: Hardware"))
            .default_pos([680.0, 420.0])
            .default_width(420.0)
            .show(ctx, |ui| hardware_ui(ui, machine));
    }

    fn controls_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, running: &mut bool) {
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

    fn registers_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, running: bool) {
        let editable = !running;
        egui::Grid::new("dbg_regs_grid").num_columns(2).show(ui, |ui| {
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
            ui.add_enabled(
                editable,
                egui::DragValue::new(&mut machine.cpu.s).hexadecimal(4, false, true),
            );
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

    fn disasm_ui(&mut self, ui: &mut egui::Ui, machine: &Machine) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.disasm_follow_pc, "Follow PC");
            ui.add(egui::TextEdit::singleline(&mut self.disasm_goto_text).desired_width(60.0));
            if ui.button("Go").clicked()
                && let Some(addr) = parse_addr(&self.disasm_goto_text)
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

    fn memory_ui(&mut self, ui: &mut egui::Ui, machine: &mut Machine, running: bool) {
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

    fn stack_ui(&self, ui: &mut egui::Ui, machine: &Machine) {
        ui.label(format!("S = ${:04X}", machine.cpu.s));
        egui::ScrollArea::vertical()
            .id_salt("dbg_stack_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                egui::Grid::new("dbg_stack_grid").striped(true).show(ui, |ui| {
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

/// Hardware-state panel: GIME `$FF90`-`$FF9F` decode, MMU task/bank map,
/// PIA0/PIA1 port state, and cartridge-port line states. Read-only (nothing
/// here is meaningfully "editable" — these are latched/derived hardware
/// states, not CPU-visible registers with obvious edit semantics) and, apart
/// from [`coco_core::Machine::video_mode_summary`] and
/// [`coco_core::SystemBus::peek`]-free field reads, touches only `pub` struct
/// fields already exposed by `coco-core` — no new side-effect-free read paths
/// were needed for this panel.
fn hardware_ui(ui: &mut egui::Ui, machine: &Machine) {
    let g = &machine.bus.gime;
    ui.label(machine.video_mode_summary());
    ui.separator();

    ui.label("INIT0 ($FF90)");
    ui.horizontal(|ui| {
        for (label, bit) in [
            ("COCO", init0::COCO),
            ("MMUEN", init0::MMUEN),
            ("IEN", init0::IEN),
            ("FEN", init0::FEN),
            ("MC3", init0::MC3),
            ("MC2", init0::MC2),
            ("MC1", init0::MC1),
            ("MC0", init0::MC0),
        ] {
            ui.label(format!("{label}:{}", u8::from(g.init0 & bit != 0)));
        }
    });
    ui.label("INIT1 ($FF91)");
    ui.horizontal(|ui| {
        ui.label(format!("TINS:{}", u8::from(g.init1 & init1::TINS != 0)));
        ui.label(format!("TR:{}", u8::from(g.init1 & init1::TR != 0)));
    });
    ui.separator();

    ui.label(format!(
        "IRQ  enable:${:02X} pending:${:02X} (IEN={})",
        g.irq_enable,
        g.irq_pending,
        g.init0 & init0::IEN != 0
    ));
    ui.label(format!(
        "FIRQ enable:${:02X} pending:${:02X} (FEN={})",
        g.firq_enable,
        g.firq_pending,
        g.init0 & init0::FEN != 0
    ));
    ui.label(format!(
        "Timer reload:${:04X} count:${:04X} fast-clock:{}",
        g.timer_reload,
        g.timer_count,
        g.timer_is_fast()
    ));
    ui.separator();

    ui.label(format!(
        "MMU enabled:{} active task:{}",
        g.mmu_enabled, g.task
    ));
    for task in 0..gime::TASK_COUNT {
        let blocks: Vec<String> = g.mmu[task].iter().map(|b| format!("{b:02X}")).collect();
        ui.label(format!("  task {task}: {}", blocks.join(" ")));
    }
    ui.separator();

    pia_ui(ui, "PIA0", &machine.bus.pia0);
    pia_ui(ui, "PIA1", &machine.bus.pia1);
    ui.separator();

    ui.label(format!(
        "Cart lines: HALT*={} CART*-ties-Q={} NMI-pending={}",
        machine.bus.halt_asserted(),
        machine.bus.cart.cart_line_ties_q(),
        machine.bus.cart.nmi_pending(),
    ));
}

fn pia_ui(ui: &mut egui::Ui, name: &str, pia: &MC6821) {
    ui.label(name);
    egui::Grid::new(format!("dbg_{name}_grid")).show(ui, |ui| {
        ui.label("");
        ui.label("output");
        ui.label("ddr");
        ui.label("control");
        ui.label("input");
        ui.label("C1 flag");
        ui.end_row();
        for (label, port) in [("A", &pia.a), ("B", &pia.b)] {
            ui.label(label);
            ui.label(format!("${:02X}", port.output));
            ui.label(format!("${:02X}", port.ddr));
            ui.label(format!("${:02X}", port.control));
            ui.label(format!("${:02X}", port.input));
            ui.label(format!("{}", port.control & cr::C1_FLAG != 0));
            ui.end_row();
        }
    });
}

/// Parse a user-typed hex address (`$C000`, `0xC000`, or bare `C000`) to a
/// 16-bit CPU logical address.
fn parse_addr(text: &str) -> Option<u16> {
    u16::from_str_radix(strip_hex_prefix(text), 16).ok()
}

/// Same as [`parse_addr`] but for a (possibly wider than 64K) physical byte
/// offset, for the Memory panel's Physical-RAM goto box.
fn parse_addr_u32(text: &str) -> Option<u32> {
    u32::from_str_radix(strip_hex_prefix(text), 16).ok()
}

fn strip_hex_prefix(text: &str) -> &str {
    let t = text.trim();
    t.strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t)
}

/// Printable-ASCII rendering for the Memory panel's ASCII column; non-
/// printable bytes show as `.`, matching every other hex-dump tool.
fn ascii_char(byte: u8) -> char {
    if (0x20..=0x7E).contains(&byte) {
        byte as char
    } else {
        '.'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn load_rom() -> Box<[u8]> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
            .into_boxed_slice()
    }

    fn boot_machine() -> Machine {
        Machine::new(coco_core::MachineConfig::default(), load_rom())
    }

    #[test]
    fn parse_addr_accepts_dollar_0x_and_bare_hex() {
        assert_eq!(parse_addr("$C000"), Some(0xC000));
        assert_eq!(parse_addr("0xC000"), Some(0xC000));
        assert_eq!(parse_addr("C000"), Some(0xC000));
        assert_eq!(parse_addr(" c000 "), Some(0xC000));
        assert_eq!(parse_addr("not hex"), None);
    }

    #[test]
    fn ascii_char_dots_non_printable() {
        assert_eq!(ascii_char(b'A'), 'A');
        assert_eq!(ascii_char(0x00), '.');
        assert_eq!(ascii_char(0x7F), '.');
    }

    /// `run_field` with no breakpoints/watchpoints must behave exactly like
    /// `Machine::run_field` — the zero-overhead common case the whole
    /// per-frame running loop depends on.
    #[test]
    fn run_field_with_no_breakpoints_matches_plain_run_field() {
        let mut via_panel = boot_machine();
        let mut via_plain = boot_machine();
        let mut panel = DebuggerPanel::new();

        for _ in 0..3 {
            assert!(panel.run_field(&mut via_panel), "no breakpoints set: must always continue");
            via_plain.run_field();
        }

        assert_eq!(via_panel.cpu.pc, via_plain.cpu.pc);
        assert_eq!(via_panel.cpu.cycles, via_plain.cpu.cycles);
    }

    /// An enabled breakpoint stops `run_field` early (returns `false`) and
    /// parks the CPU exactly at the breakpoint address.
    #[test]
    fn run_field_stops_at_breakpoint() {
        const PROBE_STEPS: usize = 40;
        let target = {
            let mut probe = boot_machine();
            for _ in 0..PROBE_STEPS {
                probe.step_instruction();
            }
            probe.cpu.pc
        };

        let mut machine = boot_machine();
        let mut panel = DebuggerPanel::new();
        panel.core.add_breakpoint(target);

        let mut stopped = false;
        for _ in 0..3 {
            if !panel.run_field(&mut machine) {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "breakpoint should have stopped a run_field call");
        assert_eq!(machine.cpu.pc, target);
    }

    /// Step In always retires exactly one real instruction, never stopping
    /// mid-HALT-burn.
    #[test]
    fn step_in_advances_pc() {
        let mut machine = boot_machine();
        let pc0 = machine.cpu.pc;
        DebuggerPanel::step_in(&mut machine);
        assert_ne!(machine.cpu.pc, pc0);
    }

    /// Step Over on a non-call instruction falls back to Step In (advances
    /// exactly one instruction, same as `step_in_advances_pc`).
    #[test]
    fn step_over_falls_back_to_step_in_for_non_call() {
        // Cold start's first instruction is not a JSR/BSR/LBSR (verified by
        // `step_in_advances_pc` passing on the same boot state); step_over
        // must still advance exactly like step_in would.
        let mut via_over = boot_machine();
        let mut via_in = boot_machine();
        let mut panel = DebuggerPanel::new();
        panel.step_over(&mut via_over);
        DebuggerPanel::step_in(&mut via_in);
        assert_eq!(via_over.cpu.pc, via_in.cpu.pc);
    }

    /// Step Scanline advances the scanline counter (or wraps the field) and
    /// makes forward progress.
    #[test]
    fn step_scanline_advances_scanline_or_field() {
        let mut machine = boot_machine();
        let start_line = machine.current_scanline();
        let pc0 = machine.cpu.pc;
        DebuggerPanel::step_scanline(&mut machine);
        assert!(machine.current_scanline() != start_line || machine.cpu.pc != pc0);
    }
}
