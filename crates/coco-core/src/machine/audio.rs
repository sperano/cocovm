//! The audio grid bridge: renders each scanline's latched-input events into
//! the oversampled stereo sample grid the frontend drains
//! (`docs/plan-audio-pipeline.md`).

use crate::audio;

use super::Machine;

impl Machine {
    /// Drain the speaker samples accumulated since the last call (one per
    /// scanline, i.e. lines-per-field × field-rate ≈ 15.7 kHz).
    pub fn take_audio(&mut self) -> std::vec::Drain<'_, [f32; 2]> {
        self.audio_buffer.drain(..)
    }

    /// The audio sample rate matching [`Machine::take_audio`]'s stream: the
    /// oversampled grid rate, [`audio::OVERSAMPLE`] × the scanline rate.
    pub fn audio_sample_rate(&self) -> f64 {
        self.line_rate() * f64::from(audio::OVERSAMPLE)
    }

    /// Scanlines per second (~15.7 kHz NTSC) — the audio grid's line clock.
    fn line_rate(&self) -> f64 {
        self.config.video.lines_per_field() as f64 * self.config.video.field_rate_hz()
    }

    /// Render the scanline that just executed to [`audio::OVERSAMPLE`]
    /// stereo grid samples (`docs/plan-audio-pipeline.md`).
    ///
    /// Latched inputs replay from the cycle-timestamped events the bus
    /// recorded during the line: each grid slot holds the state in effect
    /// at its start (a level change mid-slot lands on the next slot — grid
    /// resolution, the documented quantization). Generators are sampled
    /// per slot: the mux-gated cartridge input
    /// ([`crate::cart::Cartridge::audio_sample`] — the AY drains a quarter-line
    /// of accumulated output) and the crystal PSG pair
    /// ([`crate::cart::Cartridge::generator_sample`], wall-clock `dt` so the GIME
    /// double-speed poke can't retune them). The cassette level is sampled
    /// once per line — its 1200/2400 Hz square wave is far below even the
    /// line rate.
    pub(super) fn flush_line_audio(&mut self) {
        let line_start = self.audio_line_start;
        let line_end = self.bus.cycle_clock;
        self.audio_line_start = line_end;
        // A HALT-free line spans `line_budget` cycles; keep the real span so
        // event timestamps land in the right slot even on odd lines.
        let span = line_end.saturating_sub(line_start).max(1);
        let slot_dt = 1.0 / self.audio_sample_rate();
        let cassette_bit = self.bus.cassette.playing() && self.bus.cassette.input_bit();

        let events = std::mem::take(&mut self.bus.audio_events);
        let mut inputs = self.audio_line_inputs;
        let mut cursor = 0;
        for k in 0..u64::from(audio::OVERSAMPLE) {
            let slot_start = line_start + span * k / u64::from(audio::OVERSAMPLE);
            while cursor < events.len() && events[cursor].cycle <= slot_start {
                inputs = events[cursor].inputs;
                cursor += 1;
            }
            let ay = self.bus.cart.audio_sample();
            let generators = self.bus.cart.generator_sample(slot_dt);
            self.audio_buffer
                .push(audio::mix(&inputs, cassette_bit, ay, generators));
        }
        // Events in the final slot's tail take effect from the next line's
        // first slot: the bus's current state is the next line's start state.
        self.audio_line_inputs = self.bus.audio_inputs;
    }
}
