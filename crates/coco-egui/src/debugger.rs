//! Interactive debugger UI: Controls, Registers,
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
//! tripped breakpoint or watchpoint pauses the emulator through the same
//! `running` flag the debugger's own Run/Pause control drives, rather than
//! needing a second "why did we stop"
//! flag. Every read view (disassembly, memory, stack) goes through
//! [`coco_core::SystemBus::peek`] — never `read` — so simply having the
//! debugger open can never perturb PIA/GIME/cart state (
//! §2, "side-effect-free reads").
//!
//! Widgets are editable only while paused (`ui.add_enabled(!running, ..)`);
//! while running the panels still redraw every frame from live `peek`s, same
//! as the main screen.

use coco_core::debug::Debugger;
use coco_core::debug::StopReason;
use coco_core::{Machine, StepKind};
use eframe::egui;
use mc6809::disasm::disassemble;

mod controls;
mod disasm;
mod hardware;
mod memory;
mod registers;
mod stack;

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
    /// caller can pause (`self.running = false`), same as the debugger
    /// panel's own Pause control.
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
            if matches!(
                machine.step_instruction().kind,
                StepKind::Instruction { .. }
            ) {
                return;
            }
        }
    }

    /// Step Over: temp-breakpoints past a call instruction (JSR/BSR/LBSR) so
    /// the callee runs to completion in one step; falls back to Step In for
    /// every other opcode.
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
    /// — i.e. until the enclosing subroutine's
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
            .show(ctx, |ui| hardware::hardware_ui(ui, machine));
    }
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
#[path = "debugger_test.rs"]
mod tests;
