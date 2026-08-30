//! The audio grid bridge: renders each scanline's latched-input events into
//! the oversampled stereo sample grid the frontend drains.

use crate::audio;

use super::Machine;

impl Machine {
    /// Drain the speaker samples accumulated since the last call (one per
    /// scanline, that is, lines-per-field × field-rate ≈ 15.7 kHz).
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

    /// Render the most recently executed scanline to [`audio::OVERSAMPLE`] stereo
    /// grid samples, replaying latched bus events per slot (grid quantization).
    pub(super) fn flush_line_audio(&mut self) {
        let line_start = self.audio_line_start;
        let line_end = self.bus.cycle_clock;
        self.audio_line_start = line_end;
        // Keep the real span so event timestamps land in the right slot on odd lines.
        let span = line_end.saturating_sub(line_start).max(1);
        let sample_rate = self.audio_sample_rate();
        let slot_dt = 1.0 / sample_rate;
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
            let mux = self
                .audio_mux
                .sample(&inputs, cassette_bit, ay, sample_rate);
            self.audio_buffer
                .push(audio::mix_direct(mux, &inputs, generators));
        }
        // Final-slot-tail events carry over as next line's start state.
        self.audio_line_inputs = self.bus.audio_inputs;
    }
}
