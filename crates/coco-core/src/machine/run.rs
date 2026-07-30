//! The scanline-driven run loop: `run_field`/`step_instruction` (the
//! resumable per-instruction primitive both are built on), the per-cycle
//! peripheral tick, the per-line trailer, and interrupt servicing.

use crate::config::MachineVariant;

use super::{Machine, StepEvent, StepKind, CPU_HZ, FAST_TIMER_TICKS_PER_CPU_CYCLE};

impl Machine {
    /// Run one video field's worth of emulation (`DESIGN.md` §4).
    ///
    /// Scanline-driven: each line runs a slice of CPU cycles then pulses the
    /// horizontal sync. The field (vertical) sync is two separate edges at
    /// their own scanlines mid-field — not one pulse at the end of the loop
    /// — per [`crate::config::VideoStandard::fs_falling_line`] /
    /// [`crate::config::VideoStandard::fs_rising_line`]. All of these are wired to
    /// PIA0 and drive the CPU IRQ, which is delivered between instructions —
    /// this is what breaks the stock ROM out of its idle loop and runs
    /// BASIC's housekeeping. Video scanout is filled at the end (`§6`).
    pub fn run_field(&mut self) {
        // Resumable equivalent of the old nested scanline/cycle loops: drive
        // `step_instruction` from wherever the machine is parked until a field
        // completes. On a fresh machine (or right after any previous field —
        // both leave `line`/`line_cycles_spent` at 0) this is exactly one full
        // field from line 0, byte-for-byte identical to the pre-refactor loop.
        while !self.step_instruction().field_complete {}
    }

    /// Execute exactly one instruction (or one burned HALT* cycle) with full
    /// fidelity — peripheral ticks, NMI/FIRQ/IRQ servicing, and, when this step
    /// crosses the current scanline's cycle budget, the per-line trailer
    /// (hsync, the two field-sync edges at their lines, one audio sample, the
    /// GIME timer tick) and the field wrap (render + `field_complete`). This is
    /// the single primitive `run_field` and the debugger's `run_until` are both
    /// built on; the two together reproduce the old `run_field`/`run_cycles`
    /// nested loops one step at a time.
    ///
    /// Ordering is load-bearing and preserved exactly from the old code (see
    /// [`Machine::step_cpu_unit`] and [`Machine::end_of_line`]): the per-line
    /// trailer runs immediately after the instruction that pushes the line over
    /// budget, in the same call, before any instruction of the next line.
    pub fn step_instruction(&mut self) -> StepEvent {
        let lines = self.config.video.lines_per_field();
        loop {
            // Sample this line's budget once, at its start — same point the old
            // `run_field` sampled `cycles_per_field()/lines`.
            if self.line_cycles_spent == 0 {
                self.line_budget = self.cycles_per_field() / lines;
            }
            // Mirror `run_cycles`' `while spent < budget`: run one CPU unit if
            // the line still has budget. For real video timing `line_budget` is
            // always well above one instruction, so this branch always runs;
            // the `else` only guards a degenerate zero-budget line.
            if self.line_cycles_spent < self.line_budget {
                let (cycles, was_instruction) = self.step_cpu_unit();
                self.line_cycles_spent += cycles;
                let kind = if was_instruction {
                    StepKind::Instruction { cycles }
                } else {
                    StepKind::HaltCycle
                };
                // `run_cycles` exits when spent >= budget; `run_field` then runs
                // the per-line trailer. A single CPU unit (≤ ~20 cycles) can
                // cross at most one ~57-cycle line boundary, so one trailer
                // suffices.
                let field_complete = if self.line_cycles_spent >= self.line_budget {
                    let done = self.end_of_line();
                    self.line_cycles_spent = 0;
                    done
                } else {
                    false
                };
                return StepEvent { kind, field_complete };
            }
            // Degenerate zero-budget line (never reached for real timing): no
            // CPU unit to run — do the trailer and continue to the next line so
            // every call still makes forward progress.
            let field_complete = self.end_of_line();
            self.line_cycles_spent = 0;
            if field_complete {
                return StepEvent { kind: StepKind::HaltCycle, field_complete: true };
            }
        }
    }

    /// One iteration of the old `run_cycles` inner loop: burn a HALT* cycle or
    /// execute one instruction, then tick the per-cycle peripherals. Returns
    /// `(cycles, was_instruction)`.
    ///
    /// The cartridge HALT* line has priority over everything (MC6809 pin
    /// behaviour): while a device holds it — the FD-502's sector-transfer
    /// handshake — the CPU sits at an instruction boundary burning cycles and
    /// pending interrupts wait. The cartridge is ticked either way so it can
    /// pace the very work (DRQ cadence) that releases the line.
    ///
    /// The MC6809 recognizes interrupts only at the *end* of an instruction, so
    /// the first instruction after HALT* releases must execute before any
    /// pending NMI/IRQ/FIRQ is serviced. Skipping this lets the completion NMI
    /// of an FD-502 sector read preempt the DSKCON copy loop's `STB ,X+` that
    /// stores the sector's final byte — dropping one byte per sector on load.
    fn step_cpu_unit(&mut self) -> (u32, bool) {
        let (cycles, was_instruction) = if self.bus.halt_asserted() {
            self.prev_halted = true;
            (1, false)
        } else {
            // Coming straight out of HALT, run one instruction before
            // acknowledging interrupts (they stay pending for next loop).
            if !self.prev_halted {
                self.bus.poll_cart_interrupt();
                if self.bus.take_nmi() {
                    self.cpu.nmi(&mut self.bus);
                }
                self.service_interrupts();
            }
            self.prev_halted = false;
            (self.cpu.step(&mut self.bus), true)
        };
        self.bus.cart.tick(cycles);
        self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
        self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());
        self.bus.cycle_clock = self.bus.cycle_clock.wrapping_add(u64::from(cycles));
        (cycles, was_instruction)
    }

    /// The per-scanline trailer from the old `run_field` loop body, run after
    /// the current line's cycle budget is spent: horizontal sync, the two
    /// field-sync edges when `line` matches, one speaker sample, and the GIME
    /// interval-timer tick. Advances `line`; at the end of the field it wraps
    /// to 0, renders the framebuffer, and returns `true`.
    pub(super) fn end_of_line(&mut self) -> bool {
        let lines = self.config.video.lines_per_field();
        let fs_falling_line = self.config.video.fs_falling_line(self.config.variant);
        let fs_rising_line = self.config.video.fs_rising_line(self.config.variant);
        self.bus.hsync();
        if self.line == fs_falling_line {
            self.bus.fs_falling();
        }
        if self.line == fs_rising_line {
            self.bus.fs_rising();
        }
        self.render_scanline();
        // Render this line's audio to the oversampled stereo grid,
        // self-capping when nothing drains it.
        if self.audio_buffer.len() >= super::AUDIO_BUFFER_CAP {
            self.audio_buffer.clear();
        }
        self.flush_line_audio();
        // GIME interval timer: TINS=1 counts the fixed 3.58 MHz clock — 4 ticks
        // per normal-speed CPU cycle, 2 per double-speed cycle — TINS=0 counts
        // horizontal syncs (1 per line). No such timer exists on the plain-SAM
        // path (CoCo 1/2) — the GIME stays completely inert there
        // (`docs/coco12-plan.md` Phase 4). `line_budget` is this line's sampled
        // cycle count — the old loop's `cycles_per_line`.
        if self.config.variant == MachineVariant::Coco3 {
            let ticks = if self.bus.gime.timer_is_fast() {
                let per_cycle =
                    FAST_TIMER_TICKS_PER_CPU_CYCLE / if self.bus.gime.cpu_fast { 2 } else { 1 };
                self.line_budget * per_cycle
            } else {
                1
            };
            self.bus.gime.tick_timer(ticks);
        }
        self.line += 1;
        if self.line >= lines {
            self.line = 0;
            self.render_field();
            true
        } else {
            false
        }
    }

    /// Deliver pending FIRQ/IRQ to the CPU. The CPU itself honours the F/I masks
    /// and leaves a masked, still-asserted line pending for the next check.
    fn service_interrupts(&mut self) {
        if self.bus.firq_asserted() {
            self.cpu.firq(&mut self.bus);
        }
        if self.bus.irq_asserted() {
            self.cpu.irq(&mut self.bus);
        }
    }

    fn cycles_per_field(&self) -> u32 {
        // Speed-poke source differs per variant: the GIME's own R1 latch on
        // CoCo 3, the plain SAM's R0|R1 strobes on CoCo 1/2
        // (`docs/coco12-plan.md` Phase 4; `SAM::cpu_fast`'s KNOWN GAP note).
        let cpu_fast = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.cpu_fast,
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.cpu_fast(),
        };
        let hz = if cpu_fast { CPU_HZ * 2.0 } else { CPU_HZ };
        (hz / self.config.video.field_rate_hz()) as u32
    }
}
