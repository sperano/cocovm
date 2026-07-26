//! Latched audio-input snapshotting and event recording (`crate::audio`):
//! bridges the PIA/cartridge register state into the audio pipeline's
//! event-timestamped grid.

use super::SystemBus;

impl SystemBus {
    /// Snapshot the latched audio-affecting inputs (`crate::audio`): the
    /// 6-bit DAC, single-bit beeper, SNDEN + mux selects, cassette relay,
    /// and the cartridge's latched stereo outputs. (Tandy Service Manual
    /// mux table via MAME `coco.cpp` `update_sound`; SEB Unravelled II
    /// $FF22/$FF23. Generator-type sources — the mux-10 AY path and
    /// crystal PSGs — are sampled at flush time instead, see
    /// `Machine::flush_line_audio`.)
    fn snapshot_audio_inputs(&self) -> crate::audio::AudioInputs {
        /// PIA1 PB1: the single-bit sound output.
        const SINGLE_BIT: u8 = 0x02;
        let (cart_left, cart_right) = self.cart.sound_levels();
        crate::audio::AudioInputs {
            dac: (self.pia1.a.output & self.pia1.a.ddr & 0xFC) >> 2,
            single_bit: self.pia1.b.output & self.pia1.b.ddr & SINGLE_BIT != 0,
            snden: self.pia1.b.c2_output(),
            sel: u8::from(self.pia0.b.c2_output()) << 1 | u8::from(self.pia0.a.c2_output()),
            cassette_relay: self.pia1.a.c2_output(),
            cart_left,
            cart_right,
        }
    }

    /// Mix one stereo sample from the CURRENT latched inputs plus one
    /// `dt`-second generator step — the instantaneous speaker level, for
    /// tests and level meters. The machine's real audio path renders the
    /// event-timestamped grid instead (`Machine::flush_line_audio`); this
    /// probe advances the generator clocks (AY drain, PSG crystals) as a
    /// side effect exactly like one grid slot does.
    pub fn sound_probe(&mut self, dt: f64) -> [f32; 2] {
        let inputs = self.snapshot_audio_inputs();
        let cassette_bit = self.cassette.playing() && self.cassette.input_bit();
        let ay = self.cart.audio_sample();
        let generators = self.cart.generator_sample(dt);
        crate::audio::mix(&inputs, cassette_bit, ay, generators)
    }

    /// Record a cycle-timestamped audio event if the write that just landed
    /// changed any latched audio input. Called on the PIA and
    /// cartridge-window write paths only, and cheap even there: one
    /// snapshot + compare per write.
    pub(super) fn note_audio_write(&mut self) {
        let inputs = self.snapshot_audio_inputs();
        if inputs != self.audio_inputs {
            self.audio_inputs = inputs;
            self.audio_events.push(crate::audio::AudioEvent {
                cycle: self.cycle_clock,
                inputs,
            });
        }
    }
}
