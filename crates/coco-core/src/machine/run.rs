//! The scanline-driven run loop: `run_field`/`step_instruction` (the
//! resumable per-instruction primitive both are built on), the per-cycle
//! peripheral tick, the per-line trailer, and interrupt servicing.

use crate::config::MachineVariant;

use super::{CPU_HZ, FAST_TIMER_TICKS_PER_CPU_CYCLE, Machine, StepEvent, StepKind};

impl Machine {
    /// Run one video field's worth of emulation (`DESIGN.md` §4). Scanline-driven:
    /// each line runs a CPU slice then pulses hsync/field-sync, which drive the CPU IRQ.
    pub fn run_field(&mut self) {
        while !self.step_instruction().field_complete {}
    }

    /// Execute exactly one instruction (or one burned HALT* cycle): peripheral
    /// ticks, interrupt servicing, and — when it crosses the line's cycle
    /// budget — the per-line trailer, run before the next line's first
    /// instruction.
    pub fn step_instruction(&mut self) -> StepEvent {
        let lines = self.config.video.lines_per_field();
        loop {
            // Sample this line's budget once, at its start.
            if self.line_cycles_spent == 0 {
                self.line_budget = self.cycles_per_field() / lines;
            }
            // The `else` branch only guards a degenerate zero-budget line;
            // real timing always takes this branch.
            if self.line_cycles_spent < self.line_budget {
                let (cycles, was_instruction) = self.step_cpu_unit();
                self.line_cycles_spent += cycles;
                let kind = if was_instruction {
                    StepKind::Instruction { cycles }
                } else {
                    StepKind::HaltCycle
                };
                // A single CPU unit crosses at most one line boundary, so one trailer suffices.
                let field_complete = if self.line_cycles_spent >= self.line_budget {
                    let done = self.end_of_line();
                    self.line_cycles_spent = 0;
                    done
                } else {
                    false
                };
                return StepEvent {
                    kind,
                    field_complete,
                };
            }
            // Degenerate zero-budget line (never reached for real timing): run the trailer.
            let field_complete = self.end_of_line();
            self.line_cycles_spent = 0;
            if field_complete {
                return StepEvent {
                    kind: StepKind::HaltCycle,
                    field_complete: true,
                };
            }
        }
    }

    /// Burn a HALT* cycle or execute one instruction, then tick per-cycle
    /// peripherals. The instruction right after a HALT* release must run
    /// before servicing any pending interrupt — otherwise an FD-502
    /// completion NMI can drop the sector's final byte.
    fn step_cpu_unit(&mut self) -> (u32, bool) {
        let (cycles, was_instruction) = if self.bus.halt_asserted() {
            self.prev_halted = true;
            (1, false)
        } else {
            let cycles_before = self.cpu.cycles;
            // Coming straight out of HALT, run one instruction before acknowledging interrupts.
            if !self.prev_halted {
                self.bus.poll_cart_interrupt();
                if self.bus.take_nmi() {
                    self.cpu.nmi(&mut self.bus);
                }
                self.service_interrupts();
            }
            self.prev_halted = false;
            self.cpu.step(&mut self.bus);
            let elapsed = self.cpu.cycles - cycles_before;
            let cycles = u32::try_from(elapsed).expect("one CPU unit must fit in u32 cycles");
            (cycles, true)
        };
        self.bus.cart.tick(cycles);
        self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
        self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());
        self.bus.cycle_clock = self.bus.cycle_clock.wrapping_add(u64::from(cycles));
        (cycles, was_instruction)
    }

    /// Per-scanline trailer run after the line's cycle budget is spent: hsync,
    /// field-sync edges, an audio sample, and the GIME timer tick. Returns
    /// `true` when the field wraps.
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
        // Render this line's audio to the oversampled stereo grid; self-caps
        // when nothing drains it.
        if self.audio_buffer.len() >= super::AUDIO_BUFFER_CAP {
            self.audio_buffer.clear();
        }
        self.flush_line_audio();
        // GIME interval timer: TINS=1 counts a fixed clock, TINS=0 counts hsyncs.
        // No such timer exists on CoCo 1/2 — the GIME stays inert there.
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
        // Speed-poke source differs per variant: GIME's R1 latch (CoCo 3) vs
        // SAM's R0|R1 strobes (CoCo 1/2).
        let cpu_fast = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.cpu_fast,
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.cpu_fast(),
        };
        let hz = if cpu_fast { CPU_HZ * 2.0 } else { CPU_HZ };
        (hz / self.config.video.field_rate_hz()) as u32
    }
}

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
