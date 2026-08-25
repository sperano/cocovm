//! AY-3-8913 programmable sound generator (PSG) core: the classic AY-3-8910
//! tone/noise/envelope generators and DAC, minus the two I/O ports (R14/R15
//! are present in the register file — a real chip's register latch still
//! addresses all 16 — but have no pins on the AY-3-8913 package, so writes to
//! them are stored and otherwise inert).
//!
//! No bus/CPU dependencies: a standalone register-level model driven by
//! [`AY8913::write_reg`]/[`AY8913::read_reg`] and stepped by master clocks via
//! [`AY8913::step`]. Mirrors MAME `src/devices/sound/ay8910.cpp`
//! (`ay8910_device`) in its classic, non-expanded (AY8930), non-YM2149 mode —
//! every fact below is cited against that file.
//!
//! **Deviation from MAME**: real AY output mixing combines the three
//! channels through a shared resistor network (MAME's `mix_3D`, indexed by
//! an `8*32*32*32`-entry precomputed table) — a genuinely nonlinear
//! combination. [`AY8913::drain`] instead sums the three channels' already
//! gated, already-DAC'd levels and divides by three (`SINGLE_OUTPUT` style,
//! per `docs/ssc-spec.md`), which keeps full-scale output comparable
//! regardless of how many channels are active but does not reproduce the
//! real chip's channel-interaction nonlinearity. Good enough for a sound
//! cartridge's music/SFX; not bit-accurate against a chip analyzer capture.

use serde::{Deserialize, Serialize};

mod envelope;

use envelope::Envelope;

/// Register indices (MAME `ay8910.h`'s register-id enum). Only 0–13 have any
/// effect; 14/15 (`AY_PORTA`/`AY_PORTB`) are stored but inert — no I/O pins on
/// the AY-3-8913.
pub mod reg {
    pub const TONE_A_FINE: u8 = 0x00;
    pub const TONE_A_COARSE: u8 = 0x01;
    pub const TONE_B_FINE: u8 = 0x02;
    pub const TONE_B_COARSE: u8 = 0x03;
    pub const TONE_C_FINE: u8 = 0x04;
    pub const TONE_C_COARSE: u8 = 0x05;
    pub const NOISE_PERIOD: u8 = 0x06;
    pub const MIXER: u8 = 0x07;
    pub const VOL_A: u8 = 0x08;
    pub const VOL_B: u8 = 0x09;
    pub const VOL_C: u8 = 0x0A;
    pub const ENV_FINE: u8 = 0x0B;
    pub const ENV_COARSE: u8 = 0x0C;
    pub const ENV_SHAPE: u8 = 0x0D;
    /// Total addressable registers — the latch is 4 bits (0-15) even though
    /// only 0-13 do anything.
    pub const COUNT: usize = 16;
}

/// Mixer register (R7) bit layout: bits 0-2 tone disable A/B/C, bits 3-5
/// noise disable A/B/C — all *active-low enable* (bit set = that
/// generator's contribution is disabled for the channel). Bits 6-7 select
/// I/O port direction on a real AY-3-8910; ignored here (no ports).
///
/// `pub(crate)` so `crate::ssc`'s sound-data engine can read-modify-write R7
/// through the same named constants instead of duplicating the bit shifts.
pub(crate) mod mixer {
    pub const TONE_DISABLE_SHIFT: u8 = 0;
    pub const NOISE_DISABLE_SHIFT: u8 = 3;
}

/// R1/R3/R5 (tone coarse period) are only 4 bits wide in silicon — the
/// combined period is 12-bit, not 16 (MAME `is_expanded_mode() ? 0xff :
/// 0xf` on the non-expanded branch).
const TONE_COARSE_MASK: u8 = 0x0F;
/// R6 (noise period) is only 5 bits wide in silicon (MAME `noise_period()`'s
/// non-expanded `& 0x1f`).
const NOISE_PERIOD_MASK: u8 = 0x1F;
/// R8/R9/R10 (channel volume): bits 0-3 are the fixed level.
const VOL_LEVEL_MASK: u8 = 0x0F;
/// R8/R9/R10 bit 4: 1 selects envelope mode (channel follows the shared
/// envelope generator instead of its own fixed level).
const VOL_ENVELOPE_MODE: u8 = 0x10;

/// Internal generator step clock = master clock / 8 (MAME
/// `ay8910_device::device_start`: `stream_alloc(0, m_streams, master_clock /
/// 8)`). Tone/noise/envelope counters all advance one step per 8 master
/// clocks, not once per master clock.
const MASTER_CLOCK_DIVIDER: u32 = 8;

/// Envelope pacing multiplier for the classic AY-3-8910 — half the YM2149's
/// rate (MAME `m_step`: 2 for `PSG_TYPE_AY`, 1 for `PSG_TYPE_YM`). Combined
/// with the 16-level [`ENV_STEP_MASK`], a full envelope cycle takes 32
/// internal steps per level (MAME comment: "16 levels / 32 half-steps").
const ENVELOPE_STEP_MULTIPLIER: u32 = 2;

/// Envelope step counter mask/wrap value: 16 levels (classic AY-3-8910's
/// `m_env_step_mask = 0x0f`; the YM2149 doubles this to 0x1f, not modelled
/// here).
const ENV_STEP_MASK: i32 = 0x0F;

/// Non-zero LFSR seed (MAME `ay8910_reset_ym`: `m_rng = 1`) — an all-zero
/// shift register would never toggle since the feedback taps are also zero.
const NOISE_SEED: u32 = 1;

/// Channel count, for the mono-mix normalization in [`AY8913::drain`]'s
/// per-step accumulation.
const CHANNEL_COUNT: f32 = 3.0;

// ---- Volume DAC table -------------------------------------------------------
//
// The AY's per-step output voltage is set by a resistor ladder pulling
// against a shared pull-up/pull-down network, not a linear DAC. MAME derives
// its volume table from resistor values Matthew Westcott measured off a ZX
// Spectrum's AY output circuit (`ay8910.cpp`, Dec 2001) and reproduces with
// `build_single_table`.

/// Pull-down resistor common to every volume step (MAME `ay8910_param.r_down`).
const R_DOWN_OHMS: f64 = 8_000_000.0;
/// Pull-up resistor, switched in for every step except volume 0 — volume 0 is
/// a true 0V off, not merely the quietest AC level (MAME
/// `build_single_table`'s `zero_is_off`, set for `PSG_TYPE_AY`).
const R_UP_OHMS: f64 = 800_000.0;
/// Per-channel load resistor MAME's `ay8910_device` defaults to
/// (`m_res_load[chan] = 1000`).
const R_LOAD_OHMS: f64 = 1000.0;
/// Measured per-step resistor ladder, volume 0 (quietest) to 15 (loudest)
/// (MAME `ay8910.cpp` `ay8910_param.res[]`).
const STEP_RESISTORS_OHMS: [f64; 16] = [
    15950.0, 15350.0, 15090.0, 14760.0, 14275.0, 13620.0, 12890.0, 11370.0, 10600.0, 8590.0,
    7190.0, 5985.0, 4820.0, 3945.0, 3017.0, 2345.0,
];

/// Derive the 16-entry volume DAC table, normalized to `[0.0, 1.0]`.
///
/// Mirrors MAME's `build_single_table` (`ay8910.cpp`): each step's output
/// fraction `rw/rt` is the switched-on conductance ([`STEP_RESISTORS_OHMS`] +
/// [`R_LOAD_OHMS`], plus [`R_UP_OHMS`] except at level 0) over the total
/// conductance (always including [`R_DOWN_OHMS`]). We min-max normalize
/// across the 16 steps instead of MAME's legacy `-0.25 * 0.5` rescale, which
/// (since `rw/rt` is monotonic) lands volume 0 at exactly 0.0 and 15 at 1.0.
fn build_volume_table() -> [f32; 16] {
    let mut raw = [0.0f64; 16];
    for (level, &r) in STEP_RESISTORS_OHMS.iter().enumerate() {
        let mut rt = 1.0 / R_DOWN_OHMS + 1.0 / R_LOAD_OHMS + 1.0 / r;
        let mut rw = 1.0 / r;
        if level != 0 {
            rw += 1.0 / R_UP_OHMS;
            rt += 1.0 / R_UP_OHMS;
        }
        raw[level] = rw / rt;
    }
    let min = raw[0];
    let max = raw[15];
    let mut table = [0.0f32; 16];
    for (dst, &src) in table.iter_mut().zip(raw.iter()) {
        *dst = ((src - min) / (max - min)) as f32;
    }
    table
}

// ---- Tone channel ------------------------------------------------------------

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct ToneChannel {
    /// Internal-step down-counter toward the next square-wave toggle.
    count: u32,
    /// Current square-wave level.
    output: bool,
}

// ---- Core --------------------------------------------------------------------

/// The AY-3-8913 PSG: 3 tone generators, 1 noise generator, 1 envelope
/// generator, mixed to a single mono output (see module doc's "Deviation
/// from MAME").
#[derive(Serialize, Deserialize)]
pub struct AY8913 {
    regs: [u8; reg::COUNT],
    tone: [ToneChannel; 3],
    /// Noise generator's own internal-step counter, separate from the tone
    /// channels' (runs off [`reg::NOISE_PERIOD`] instead of the tone
    /// periods).
    noise_count: u32,
    /// Halves the noise period once more: the LFSR only shifts on every
    /// other period expiry (MAME `m_prescale_noise`).
    noise_prescale: bool,
    /// 17-bit LFSR state (only the low 17 bits are meaningful).
    rng: u32,
    envelope: Envelope,
    /// Fractional master-clock remainder toward the next internal step
    /// ([`MASTER_CLOCK_DIVIDER`]), carried across [`AY8913::step`] calls so a
    /// `cycles` argument that isn't a multiple of 8 doesn't lose clocks.
    clock_accum: u32,
    /// Precomputed once per instance (cheap: 16 entries) — see
    /// [`build_volume_table`]. Skipped: pure construction-time scratch,
    /// left at its `Default` (all-zero — silent, not correct) until
    /// [`AY8913::after_restore`] rebuilds it.
    #[serde(skip)]
    dac: [f32; 16],
    /// Box-filter accumulator for [`AY8913::drain`]: running sum of the
    /// per-internal-step mixed output since the last drain. Skipped:
    /// per-drain accumulator, correctly resets to zero
    ///.
    #[serde(skip)]
    sample_sum: f32,
    #[serde(skip)]
    sample_count: u32,
}

impl Default for AY8913 {
    fn default() -> Self {
        Self::new()
    }
}

impl AY8913 {
    pub fn new() -> Self {
        let mut ay = Self {
            regs: [0; reg::COUNT],
            tone: [ToneChannel::default(); 3],
            noise_count: 0,
            noise_prescale: false,
            rng: NOISE_SEED,
            envelope: Envelope::default(),
            clock_accum: 0,
            dac: build_volume_table(),
            sample_sum: 0.0,
            sample_count: 0,
        };
        // MAME's reset writes 0 to every register, so R13=0's `set_shape` side effect applies too.
        ay.envelope.set_shape(0);
        ay
    }

    /// Reset every register and generator to power-on state (MAME
    /// `ay8910_reset_ym`).
    pub fn reset(&mut self) {
        self.regs = [0; reg::COUNT];
        self.tone = [ToneChannel::default(); 3];
        self.noise_count = 0;
        self.noise_prescale = false;
        self.rng = NOISE_SEED;
        self.envelope = Envelope::default();
        self.envelope.set_shape(0);
        self.clock_accum = 0;
        self.sample_sum = 0.0;
        self.sample_count = 0;
    }

    /// Rebuilds the skipped `dac` lookup table after a snapshot restore, via
    /// [`build_volume_table`]. Idempotent.
    pub fn after_restore(&mut self) {
        self.dac = build_volume_table();
    }

    /// Write a register through the 4-bit address latch (only bits 0-3 decoded).
    /// Coarse tone/noise-period registers are masked to silicon width
    /// ([`TONE_COARSE_MASK`]/[`NOISE_PERIOD_MASK`]); other registers store the full byte.
    pub fn write_reg(&mut self, r: u8, val: u8) {
        let idx = (r & 0x0F) as usize;
        let stored = match idx as u8 {
            reg::TONE_A_COARSE | reg::TONE_B_COARSE | reg::TONE_C_COARSE => val & TONE_COARSE_MASK,
            reg::NOISE_PERIOD => val & NOISE_PERIOD_MASK,
            _ => val,
        };
        self.regs[idx] = stored;
        if idx as u8 == reg::ENV_SHAPE {
            self.envelope.set_shape(stored);
        }
    }

    /// Read a register back (the value as stored — see
    /// [`AY8913::write_reg`]'s masking).
    pub fn read_reg(&self, r: u8) -> u8 {
        self.regs[(r & 0x0F) as usize]
    }

    /// Advance the generators by `master_clocks` AY master-clock cycles
    /// (already 2× the CoCo E-clock — see `docs/ssc-spec.md`), accumulating
    /// samples for [`AY8913::drain`].
    pub fn step(&mut self, master_clocks: u32) {
        self.clock_accum += master_clocks;
        while self.clock_accum >= MASTER_CLOCK_DIVIDER {
            self.clock_accum -= MASTER_CLOCK_DIVIDER;
            self.internal_step();
        }
    }

    /// Average mixed output since the last call and reset the accumulator —
    /// a box-filter downsample since the caller samples far below the AY's internal step rate.
    pub fn drain(&mut self) -> f32 {
        let avg = if self.sample_count > 0 {
            self.sample_sum / self.sample_count as f32
        } else {
            0.0
        };
        self.sample_sum = 0.0;
        self.sample_count = 0;
        avg
    }

    fn tone_period(&self, ch: usize) -> u32 {
        let fine = self.regs[reg::TONE_A_FINE as usize + ch * 2];
        let coarse = self.regs[reg::TONE_A_COARSE as usize + ch * 2];
        u32::from(fine) | (u32::from(coarse) << 8)
    }

    fn noise_period(&self) -> u32 {
        u32::from(self.regs[reg::NOISE_PERIOD as usize])
    }

    fn env_period(&self) -> u32 {
        let fine = self.regs[reg::ENV_FINE as usize];
        let coarse = self.regs[reg::ENV_COARSE as usize];
        u32::from(fine) | (u32::from(coarse) << 8)
    }

    fn tone_disabled(&self, ch: usize) -> bool {
        self.regs[reg::MIXER as usize] & (1 << (mixer::TONE_DISABLE_SHIFT + ch as u8)) != 0
    }

    fn noise_disabled(&self, ch: usize) -> bool {
        self.regs[reg::MIXER as usize] & (1 << (mixer::NOISE_DISABLE_SHIFT + ch as u8)) != 0
    }

    /// The level (0-15) channel `ch` currently outputs when un-gated: its
    /// own fixed level, or the shared envelope's, per R8/R9/R10 bit 4.
    fn channel_level(&self, ch: usize) -> u8 {
        let vol_reg = self.regs[reg::VOL_A as usize + ch];
        if vol_reg & VOL_ENVELOPE_MODE != 0 {
            self.envelope.volume()
        } else {
            vol_reg & VOL_LEVEL_MASK
        }
    }

    /// 17-bit LFSR shift (MAME `noise_rng_tick`, "verified on AY-3-8910 and
    /// YM2149 chips"): input is bit0 XOR bit3, output is bit0.
    fn shift_noise(&mut self) {
        let bit0 = self.rng & 1;
        let bit3 = (self.rng >> 3) & 1;
        self.rng = (self.rng >> 1) | ((bit0 ^ bit3) << 16);
    }

    /// One internal step (master_clock/8): advance tone/noise/envelope, gate
    /// and sum the three channels into the running [`AY8913::drain`] average.
    fn internal_step(&mut self) {
        // Toggles every `period` steps, clamped to at least 1 to avoid an
        // infinite loop at period 0 (MAME `std::max<int>(1, tone->period)`).
        for ch_idx in 0..3 {
            let period = self.tone_period(ch_idx).max(1);
            let tone = &mut self.tone[ch_idx];
            tone.count += 1;
            while tone.count >= period {
                tone.count -= period;
                tone.output = !tone.output;
            }
        }

        // A second prescaler halves the period rate; the LFSR shifts once
        // per two prescaler toggles (MAME `sound_stream_update`).
        self.noise_count += 1;
        if self.noise_count >= self.noise_period() {
            self.noise_count = 0;
            self.noise_prescale = !self.noise_prescale;
            if !self.noise_prescale {
                self.shift_noise();
            }
        }
        let noise_out = self.rng & 1 != 0;

        // Envelope: classic AY-3-8910 rate ([`ENVELOPE_STEP_MULTIPLIER`]).
        let env_period = self.env_period() * ENVELOPE_STEP_MULTIPLIER;
        self.envelope.step_once(env_period);

        // Gates each channel by (ToneOn|ToneDisable) & (NoiseOn|NoiseDisable)
        // — MAME: both disabled means constant-1 output, not 0.
        let mut sum = 0.0f32;
        for ch in 0..3 {
            let enabled = (self.tone[ch].output || self.tone_disabled(ch))
                && (noise_out || self.noise_disabled(ch));
            let level = if enabled { self.channel_level(ch) } else { 0 };
            sum += self.dac[level as usize];
        }
        self.sample_sum += sum / CHANNEL_COUNT;
        self.sample_count += 1;
    }
}

#[cfg(test)]
#[path = "ay8913_test.rs"]
mod tests;
