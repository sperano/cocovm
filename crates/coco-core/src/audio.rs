//! Event-timestamped stereo audio pipeline.
//!
//! The old path point-sampled one mono level per scanline (~15.7 kHz), which
//! aliased software-timed DAC playback (digitized speech writes `$FF20` far
//! faster than the line rate) and quantized every level change to a line
//! boundary. This pipeline instead records a [`AudioEvent`] snapshot at the
//! CPU-cycle timestamp of every audio-affecting write, then renders each
//! scanline to a fixed [`OVERSAMPLE`]-slot stereo grid at the line's end:
//! between events the hardware holds its level (latches), so grid rendering
//! is exact reconstruction up to grid resolution, not interpolation.
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
/// SEL2:SEL1 = 01: the mux's cassette input.
const SEL_CASSETTE: u8 = 0b01;
/// SEL2:SEL1 = 10: the mux's cartridge input.
const SEL_CARTRIDGE: u8 = 0b10;

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

/// Mix one grid slot from the latched inputs, the line's cassette level, and
/// this slot's `ay` (mux-gated cartridge) and `generators` (crystal PSG) samples.
pub(crate) fn mix(
    inputs: &AudioInputs,
    cassette_bit: bool,
    ay: f32,
    generators: (f32, f32),
) -> [f32; 2] {
    let mut l = 0.0f32;
    let mut r = 0.0f32;
    if inputs.snden {
        match inputs.sel {
            0 => {
                let v = DAC_GAIN * f32::from(inputs.dac) / DAC_MAX;
                l += v;
                r += v;
            }
            SEL_CASSETTE => {
                if inputs.cassette_relay && cassette_bit {
                    l += CASSETTE_GAIN;
                    r += CASSETTE_GAIN;
                }
            }
            SEL_CARTRIDGE => {
                l += CARTRIDGE_GAIN * ay;
                r += CARTRIDGE_GAIN * ay;
            }
            _ => {} // 11: grounded
        }
    }
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
