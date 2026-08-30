//! Event-timestamped stereo audio pipeline.
//!
//! The old path point-sampled one mono level per scanline (~15.7 kHz), which
//! aliased software-timed DAC playback (digitized speech writes `$FF20` far
//! faster than the line rate) and quantized every level change to a line
//! boundary. This pipeline instead records a [`AudioEvent`] snapshot at the
//! CPU-cycle timestamp of every audio-affecting write, then renders each
//! scanline to a fixed [`OVERSAMPLE`]-slot stereo grid at the line's end:
//! between events the input latches hold their level. The MC14529 output then
//! holds while inhibited and crossfades between selected sources, matching
//! MAME's CoCo sound path and suppressing joystick-poll switching artifacts.
//!
//! Two kinds of source feed the grid:
//! - **Latched** inputs ([`AudioInputs`]): the PIA DAC, single-bit beeper,
//!   SNDEN/mux selects, and latched cartridge outputs (the Orchestra-90's
//!   stereo DACs through [`crate::cart::Cartridge::sound_levels`]). Event-recorded
//!   with cycle timestamps.
//! - **Generators**, sampled once per grid slot at flush: the mux-gated
//!   cartridge input ([`crate::cart::Cartridge::audio_sample`] — the SSC's
//!   AY-3-8913 accumulator drains a quarter-line at a time) and crystal-
//!   clocked PSGs ([`crate::cart::Cartridge::generator_sample`] — the GMC's
//!   SN76489A). The cassette level is sampled once per line: its 1200/2400 Hz
//!   square wave is far below even the line rate.

use serde::{Deserialize, Serialize};

/// Grid samples per scanline. 4 → ~62.9 kHz internal rate on NTSC; a named
/// constant per the plan — bump to 8 only if a digitized-speech title
/// measurably needs it.
pub const OVERSAMPLE: u32 = 4;

/// Relative loudness of the full-scale DAC vs the single-bit beeper.
const DAC_GAIN: f32 = 0.75;
const SINGLE_BIT_GAIN: f32 = 0.25;
/// Latched cartridge outputs (Orchestra-90 DACs) at the same full-scale
/// loudness as the internal 6-bit DAC; also applied to crystal generators
/// (GMC SN76489A).
const CART_GAIN: f32 = 0.75;
/// Tape playback through the mux: a square wave (the SALT detector's
/// output), kept below the DAC's full scale like the real attenuated level.
const CASSETTE_GAIN: f32 = 0.35;
/// Cartridge audio through the mux (state 10): matched to the DAC's gain
/// (the AY-3-8913's output is already normalized 0.0–1.0 by `Ay8913`).
const CARTRIDGE_GAIN: f32 = 0.75;
const DAC_MAX: f32 = 63.0;
/// MAME's CoCo MC14529 model uses this ramp to suppress source-switch clicks.
const MUX_CROSSFADE_SECONDS: f64 = 0.000_5;
/// SEL2:SEL1 = 01: the mux's cassette input.
const SEL_CASSETTE: u8 = 0b01;
/// SEL2:SEL1 = 10: the mux's cartridge input.
const SEL_CARTRIDGE: u8 = 0b10;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
enum MuxSource {
    #[default]
    Inhibited,
    Dac,
    Cassette,
    Cartridge,
    Grounded,
}

/// Stateful MC14529 speaker-output model. Inhibit holds the last level, and
/// source changes use MAME's CoCo crossfade to suppress switching artifacts.
/// The default adopts its first source immediately so snapshots that predate
/// this state keep their original first-sample behavior.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct AudioMux {
    output: f32,
    target: MuxSource,
    fade_from: f32,
    fade_sample: u32,
    initialized: bool,
}

impl AudioMux {
    pub fn new() -> Self {
        Self {
            initialized: true,
            ..Self::default()
        }
    }

    pub fn sample(
        &mut self,
        inputs: &AudioInputs,
        cassette_bit: bool,
        cartridge: f32,
        sample_rate: f64,
    ) -> f32 {
        let target = mux_source(inputs);
        let target_level = mux_level(target, inputs, cassette_bit, cartridge);
        if !self.initialized {
            self.output = target_level;
            self.target = target;
            self.initialized = true;
            return self.output;
        }
        if target != self.target {
            self.target = target;
            self.fade_from = self.output;
            self.fade_sample = 0;
        }
        if target == MuxSource::Inhibited {
            return self.output;
        }

        let fade_samples = (MUX_CROSSFADE_SECONDS * sample_rate).max(1.0) as u32;
        if self.fade_sample < fade_samples {
            let fraction = self.fade_sample as f32 / fade_samples as f32;
            self.output = self.fade_from + (target_level - self.fade_from) * fraction;
            self.fade_sample += 1;
        } else {
            self.output = target_level;
        }
        self.output
    }
}

/// The latched audio-affecting inputs, snapshotted on every write that
/// changes one of them (see `SystemBus::note_audio_write`).
#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub(crate) struct AudioInputs {
    /// PIA1 port A bits 2–7: the 6-bit DAC (already masked by DDR, shifted).
    pub dac: u8,
    /// PIA1 PB1 (masked by DDR): the single-bit beeper.
    pub single_bit: bool,
    /// PIA1 CB2: SNDEN, the analog mux's master gate.
    pub snden: bool,
    /// PIA0 CB2:CA2 — the mux select (SEL2:SEL1).
    pub sel: u8,
    /// PIA1 CA2: the cassette motor relay (gates the mux's cassette input).
    pub cassette_relay: bool,
    /// Latched cartridge left/right outputs (`Cartridge::sound_levels`) —
    /// the Orchestra-90's DAC pair, wire-summed by the MPI.
    pub cart_left: f32,
    pub cart_right: f32,
}

/// One latched-input change at a CPU-cycle timestamp (`SystemBus::cycle_clock`).
/// `inputs` is the state FROM this cycle onward.
#[derive(Serialize, Deserialize)]
pub(crate) struct AudioEvent {
    pub cycle: u64,
    pub inputs: AudioInputs,
}

/// Mix an instantaneous speaker probe without the machine's stateful mux
/// transition, plus this slot's crystal-clocked generator samples.
pub(crate) fn mix(
    inputs: &AudioInputs,
    cassette_bit: bool,
    ay: f32,
    generators: (f32, f32),
) -> [f32; 2] {
    let source = mux_source(inputs);
    let mux = mux_level(source, inputs, cassette_bit, ay);
    mix_direct(mux, inputs, generators)
}

/// Add the always-connected sources to a stateful mux output.
pub(crate) fn mix_direct(mux: f32, inputs: &AudioInputs, generators: (f32, f32)) -> [f32; 2] {
    let mut l = mux;
    let mut r = mux;
    if inputs.single_bit {
        l += SINGLE_BIT_GAIN;
        r += SINGLE_BIT_GAIN;
    }
    l += CART_GAIN * inputs.cart_left;
    r += CART_GAIN * inputs.cart_right;
    l += CART_GAIN * generators.0;
    r += CART_GAIN * generators.1;
    [l, r]
}

fn mux_source(inputs: &AudioInputs) -> MuxSource {
    if !inputs.snden {
        return MuxSource::Inhibited;
    }
    match inputs.sel {
        0 => MuxSource::Dac,
        SEL_CASSETTE => MuxSource::Cassette,
        SEL_CARTRIDGE => MuxSource::Cartridge,
        _ => MuxSource::Grounded,
    }
}

fn mux_level(source: MuxSource, inputs: &AudioInputs, cassette_bit: bool, cartridge: f32) -> f32 {
    match source {
        MuxSource::Dac => DAC_GAIN * f32::from(inputs.dac) / DAC_MAX,
        MuxSource::Cassette if inputs.cassette_relay && cassette_bit => CASSETTE_GAIN,
        MuxSource::Cartridge => CARTRIDGE_GAIN * cartridge,
        MuxSource::Inhibited | MuxSource::Cassette | MuxSource::Grounded => 0.0,
    }
}

#[cfg(test)]
#[path = "audio_test.rs"]
mod tests;
