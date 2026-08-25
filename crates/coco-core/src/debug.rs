//! Debug core: the [`Debugger`] the frontend owns and drives, plus the
//! side-effect-free primitives the machine exposes for it (
//! §2). The [`Debugger`] holds PC breakpoints and memory watchpoints, runs the
//! machine one instruction at a time via [`Machine::step_instruction`] until a
//! stop condition trips ([`Debugger::run_until`]), and keeps an instruction
//! trace ring for "how did I get here" / MAME trace-diffing.
//!
//! The design deliberately leaves room for the deferred features
//!: conditional breakpoints hang
//! off [`Breakpoint`], watch expressions off [`Watchpoint`].

use std::collections::{HashMap, VecDeque};

use mc6809::MC6809;

use crate::{Machine, StepKind};

/// Default depth of the instruction trace ring ([`Debugger::trace`]).
const DEFAULT_TRACE_CAP: usize = 1024;

/// Which access direction a memory watchpoint traps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WatchKind {
    /// A CPU read of the watched logical address (includes opcode/operand
    /// fetches and vector reads — the bus can't tell them apart).
    Read,
    /// A CPU write of the watched logical address.
    Write,
}

/// A watchpoint access recorded by the bus during a step: the logical address
/// touched and how. Drained by [`crate::SystemBus::take_watch_hit`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WatchHit {
    /// The CPU logical address accessed.
    pub addr: u16,
    /// Read vs write.
    pub kind: WatchKind,
}

/// The lean, enabled-only snapshot of the debugger's watchpoints installed into
/// [`crate::SystemBus`] for the duration of a [`Debugger::run_until`]. Kept
/// minimal so the per-access bus check is a single map lookup, and installed as
/// `None` when empty so a debugged run with no watchpoints keeps the bus's fast
/// path.
#[derive(Clone, Default)]
pub struct WatchTable {
    entries: HashMap<u16, WatchDirs>,
}

#[derive(Clone, Copy, Default)]
struct WatchDirs {
    read: bool,
    write: bool,
}

impl WatchTable {
    /// True when no watchpoints are present — the bus installs `None` in that
    /// case, so `read`/`write` skip the watch check entirely.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Does `addr` trap an access of direction `kind`?
    pub fn matches(&self, addr: u16, kind: WatchKind) -> bool {
        match self.entries.get(&addr) {
            Some(d) => match kind {
                WatchKind::Read => d.read,
                WatchKind::Write => d.write,
            },
            None => false,
        }
    }

    /// Add a watched address trapping the selected access directions.
    pub fn watch(&mut self, addr: u16, read: bool, write: bool) {
        self.entries.insert(addr, WatchDirs { read, write });
    }
}

/// Why [`Debugger::run_until`] stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// The PC reached an enabled breakpoint (before executing that
    /// instruction).
    Breakpoint(u16),
    /// A memory watchpoint tripped mid-instruction.
    Watchpoint {
        /// The watched logical address that was accessed.
        addr: u16,
        /// Read vs write.
        kind: WatchKind,
    },
    /// A video field completed (the machine reached a field boundary).
    FieldComplete,
    /// The instruction budget was exhausted without any of the above — the
    /// run made progress but hit no stop condition.
    Step,
}

/// A PC breakpoint. `hits` counts how many times [`Debugger::run_until`] has
/// stopped on it; `enabled` gates whether it stops at all. The struct is the
/// hook point for the deferred conditional-breakpoint feature.
#[derive(Clone, Debug)]
pub struct Breakpoint {
    /// When false the breakpoint is remembered but never stops execution.
    pub enabled: bool,
    /// Number of times a run has stopped here.
    pub hits: u64,
}

impl Default for Breakpoint {
    fn default() -> Self {
        Self {
            enabled: true,
            hits: 0,
        }
    }
}

/// A memory watchpoint on a CPU logical address. Traps reads, writes, or both.
#[derive(Clone, Debug)]
pub struct Watchpoint {
    /// Trap CPU reads of the address.
    pub read: bool,
    /// Trap CPU writes of the address.
    pub write: bool,
    /// When false the watchpoint is remembered but never stops execution.
    pub enabled: bool,
    /// Number of times a run has stopped here.
    pub hits: u64,
}

/// One entry in the instruction trace ring: the CPU register file captured
/// immediately BEFORE an instruction retired. Formatted identically to
/// `examples/trace.rs`'s `log_state` for MAME trace-diffing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub pc: u16,
    pub a: u8,
    pub b: u8,
    pub x: u16,
    pub y: u16,
    pub u: u16,
    pub s: u16,
    pub dp: u8,
    pub cc: u8,
}

impl TraceEntry {
    /// Snapshot the CPU register file (pre-instruction state).
    pub fn capture(cpu: &MC6809) -> Self {
        Self {
            pc: cpu.pc,
            a: cpu.a,
            b: cpu.b,
            x: cpu.x,
            y: cpu.y,
            u: cpu.u,
            s: cpu.s,
            dp: cpu.dp,
            cc: cpu.cc,
        }
    }

    /// One trace line in `examples/trace.rs`'s exact format (no trailing
    /// newline).
    pub fn format(&self) -> String {
        format!(
            "{:04X}:  A={:02X} B={:02X} X={:04X} Y={:04X} U={:04X} S={:04X} DP={:02X} CC={:02X}",
            self.pc, self.a, self.b, self.x, self.y, self.u, self.s, self.dp, self.cc
        )
    }
}

/// The interactive debugger the frontend owns and passes `&mut` into
/// [`Debugger::run_until`]. Holds all debug state — breakpoints, watchpoints,
/// and the trace ring — so the machine itself stays free of debug concerns
/// (the bus carries only the lean watch snapshot, and only while a run is in
/// flight).
pub struct Debugger {
    breakpoints: HashMap<u16, Breakpoint>,
    watchpoints: HashMap<u16, Watchpoint>,
    trace: VecDeque<TraceEntry>,
    trace_cap: usize,
    /// When true, record every retired instruction into the trace ring during
    /// [`Debugger::run_until`]. Off by default — the ring costs a snapshot per
    /// instruction, which a plain "run" doesn't want.
    pub trace_enabled: bool,
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}

impl Debugger {
    pub fn new() -> Self {
        Self {
            breakpoints: HashMap::new(),
            watchpoints: HashMap::new(),
            trace: VecDeque::new(),
            trace_cap: DEFAULT_TRACE_CAP,
            trace_enabled: false,
        }
    }

    /// Build a debugger with a custom trace-ring depth.
    pub fn with_trace_capacity(cap: usize) -> Self {
        Self {
            trace_cap: cap.max(1),
            ..Self::new()
        }
    }

    // ---- Breakpoints -------------------------------------------------------

    /// Add (or re-enable, resetting nothing) a PC breakpoint. Idempotent: an
    /// existing breakpoint's hit count is preserved.
    pub fn add_breakpoint(&mut self, pc: u16) {
        self.breakpoints.entry(pc).or_default().enabled = true;
    }

    /// Remove a PC breakpoint entirely (dropping its hit count).
    pub fn remove_breakpoint(&mut self, pc: u16) {
        self.breakpoints.remove(&pc);
    }

    /// Enable or disable a breakpoint without forgetting it. No-op if absent.
    pub fn set_breakpoint_enabled(&mut self, pc: u16, enabled: bool) {
        if let Some(bp) = self.breakpoints.get_mut(&pc) {
            bp.enabled = enabled;
        }
    }

    /// Inspect a breakpoint (enabled flag, hit count).
    pub fn breakpoint(&self, pc: u16) -> Option<&Breakpoint> {
        self.breakpoints.get(&pc)
    }

    /// Iterate all breakpoints as `(pc, &Breakpoint)`.
    pub fn breakpoints(&self) -> impl Iterator<Item = (u16, &Breakpoint)> {
        self.breakpoints.iter().map(|(&pc, bp)| (pc, bp))
    }

    /// Remove every breakpoint.
    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    // ---- Watchpoints -------------------------------------------------------

    /// Add or replace a memory watchpoint on `addr`, selecting trapped directions via `read`/`write`.
    /// Enabled on creation; an existing watchpoint's hit count is preserved.
    pub fn add_watchpoint(&mut self, addr: u16, read: bool, write: bool) {
        let wp = self.watchpoints.entry(addr).or_insert(Watchpoint {
            read,
            write,
            enabled: true,
            hits: 0,
        });
        wp.read = read;
        wp.write = write;
        wp.enabled = true;
    }

    /// Remove a watchpoint entirely.
    pub fn remove_watchpoint(&mut self, addr: u16) {
        self.watchpoints.remove(&addr);
    }

    /// Enable or disable a watchpoint without forgetting it. No-op if absent.
    pub fn set_watchpoint_enabled(&mut self, addr: u16, enabled: bool) {
        if let Some(wp) = self.watchpoints.get_mut(&addr) {
            wp.enabled = enabled;
        }
    }

    /// Inspect a watchpoint.
    pub fn watchpoint(&self, addr: u16) -> Option<&Watchpoint> {
        self.watchpoints.get(&addr)
    }

    /// Iterate all watchpoints as `(addr, &Watchpoint)`.
    pub fn watchpoints(&self) -> impl Iterator<Item = (u16, &Watchpoint)> {
        self.watchpoints.iter().map(|(&addr, wp)| (addr, wp))
    }

    /// Remove every watchpoint.
    pub fn clear_watchpoints(&mut self) {
        self.watchpoints.clear();
    }

    /// The lean enabled-only snapshot handed to the bus for a run.
    fn watch_table(&self) -> WatchTable {
        let mut table = WatchTable::default();
        for (&addr, wp) in &self.watchpoints {
            if wp.enabled && (wp.read || wp.write) {
                table.watch(addr, wp.read, wp.write);
            }
        }
        table
    }

    // ---- Trace ring --------------------------------------------------------

    /// The instruction trace ring, oldest first.
    pub fn trace(&self) -> impl Iterator<Item = &TraceEntry> {
        self.trace.iter()
    }

    /// Export the trace ring as text in `examples/trace.rs`'s format — one line
    /// per retired instruction — for MAME trace-diffing.
    pub fn export_trace(&self) -> String {
        let mut out = String::new();
        for entry in &self.trace {
            out.push_str(&entry.format());
            out.push('\n');
        }
        out
    }

    /// Empty the trace ring.
    pub fn clear_trace(&mut self) {
        self.trace.clear();
    }

    fn push_trace(&mut self, entry: TraceEntry) {
        if self.trace.len() == self.trace_cap {
            self.trace.pop_front();
        }
        self.trace.push_back(entry);
    }

    // ---- The run loop ------------------------------------------------------

    /// Run until a breakpoint, watchpoint, or field boundary trips, or `max_instructions` elapses.
    /// A breakpoint fires when the PC reaches it, even ahead of an interrupt that would preempt that instruction.
    pub fn run_until(&mut self, m: &mut Machine, max_instructions: u64) -> StopReason {
        m.bus.install_watches(self.watch_table());
        let reason = self.run_loop(m, max_instructions);
        m.bus.uninstall_watches();
        reason
    }

    fn run_loop(&mut self, m: &mut Machine, max_instructions: u64) -> StopReason {
        for i in 0..max_instructions {
            let pc = m.cpu.pc;
            // Skip the breakpoint check on the first iteration so a resumed run doesn't immediately re-trigger where it's parked.
            let stop_at_bp = i > 0 && matches!(self.breakpoints.get(&pc), Some(bp) if bp.enabled);
            if stop_at_bp {
                let bp = self.breakpoints.get_mut(&pc).expect("just matched above");
                bp.hits += 1;
                return StopReason::Breakpoint(pc);
            }

            m.bus.clear_watch_hit();
            let snapshot = self.trace_enabled.then(|| TraceEntry::capture(&m.cpu));
            let event = m.step_instruction();
            if let (true, Some(entry)) =
                (matches!(event.kind, StepKind::Instruction { .. }), snapshot)
            {
                self.push_trace(entry);
            }

            if let Some(hit) = m.bus.take_watch_hit() {
                if let Some(wp) = self.watchpoints.get_mut(&hit.addr) {
                    wp.hits += 1;
                }
                return StopReason::Watchpoint {
                    addr: hit.addr,
                    kind: hit.kind,
                };
            }
            if event.field_complete {
                return StopReason::FieldComplete;
            }
        }
        StopReason::Step
    }
}
