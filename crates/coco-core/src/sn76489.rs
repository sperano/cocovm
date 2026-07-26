//! TI SN76489A programmable sound generator: 3 square-wave tone channels and
//! 1 noise channel, each with a 4-bit attenuator — the music chip on the
//! Games Master Cartridge (`docs/plan-games-master-cartridge.md`).
//!
//! Every behavioural fact here is verified against MAME
//! `src/devices/sound/sn76496.cpp` (the SN76489**A** constructor variant:
//! LFSR feedback mask $10000, taps $04/$08, non-inverted output, ÷8 divider
//! on a ÷2 sample clock). MAME marks the A variant "whitenoise verified,
//! phase verified, periodic verified" — the plain SN76489's constants are
//! different ($4000/$01/$02, inverted) and are NOT what this implements.

use serde::{Deserialize, Serialize};

/// Crystal cycles per internal tone/noise tick. MAME models it as a ÷2
/// sample clock plus the SN76489A's ÷8 `clock_divider`; the net rate is one
/// counter step per 16 crystal cycles (4 MHz crystal → 250 kHz).
const CRYSTAL_CYCLES_PER_TICK: f64 = 16.0;

/// One attenuation step is 2 dB, a factor of 10^(2/20) (MAME `device_start`:
/// `out /= 1.258925412`).
const ATTENUATION_STEP: f64 = 1.258925412;

/// Full-scale amplitude of a single channel: the 4 channels sum to 1.0
/// (MAME `MAX_OUTPUT / 4`).
const CHANNEL_FULL_SCALE: f64 = 0.25;

/// Attenuation codes run 0 (loudest) to [`ATTENUATION_MUTE`] (silent —
/// `m_vol_table[15] = 0`, not just very quiet).
const ATTENUATION_MUTE: usize = 15;

/// Command-byte bit 7: set = LATCH/DATA byte (bits 6-4 pick the register,
/// bits 3-0 are data), clear = DATA-only byte for the last-latched register.
const LATCH_FLAG: u8 = 0x80;

/// A tone period register written as 0 counts as $400 (1024) — the lowest
/// frequency, not a stopped/DC output (MAME `write()` cases 0/2/4).
const PERIOD_ZERO_COUNTS_AS: u16 = 0x400;

/// Noise control register (register 6) bit 2: 1 = white noise (both LFSR
/// taps active), 0 = "periodic" noise (tap 2 forced off).
const NOISE_MODE_WHITE: u16 = 0x04;

/// Noise control bits 1-0 = 3: the noise counter mirrors tone 2's period
/// (doubled — one shift per full tone-2 output cycle) instead of a fixed
/// ÷32/64/128 rate.
const NOISE_RATE_FOLLOWS_TONE2: u16 = 0x03;

/// LFSR feedback bit for the SN76489A (`m_feedback_mask` $10000); also the
/// power-on LFSR seed (`m_RNG = m_feedback_mask`).
const LFSR_FEEDBACK: u32 = 0x10000;
/// LFSR whitenoise taps for the SN76489A (`m_whitenoise_tap1/2`).
const LFSR_TAP1: u32 = 0x04;
const LFSR_TAP2: u32 = 0x08;

/// Register indices: even = tone 0/1/2 period (10-bit), odd = tone 0/1/2 and
/// noise attenuation (4-bit), 6 = noise control (3-bit).
const REG_NOISE_CTRL: usize = 6;

/// The noise generator, viewed as a 4th channel alongside tones 0-2.
const NOISE_CHANNEL: usize = 3;

/// Build the 16-entry attenuation-code amplitude lookup ([`ATTENUATION_STEP`]
/// per code, code 15 silent) — shared by [`SN76489A::new`] and
/// [`SN76489A::after_restore`] so construction and post-snapshot-restore
/// rebuild can never drift apart.
fn build_vol_table() -> [f32; 16] {
    let mut vol_table = [0.0f32; 16];
    let mut out = CHANNEL_FULL_SCALE;
    for entry in vol_table.iter_mut().take(ATTENUATION_MUTE) {
        *entry = out as f32;
        out /= ATTENUATION_STEP;
    }
    vol_table
}

#[derive(Serialize, Deserialize)]
pub struct SN76489A {
    /// Internal tick rate: crystal / 16. Stored (not skipped) so a restored
    /// chip doesn't need the original crystal handed back in at rebuild time
    /// (`docs/plan-save-states.md`).
    tick_hz: f64,
    /// Raw register file — tone periods keep all 10 bits, attenuation and
    /// noise-control registers only ever hold their low 4 bits.
    regs: [u16; 8],
    /// Register a DATA-only byte (bit 7 clear) continues (MAME
    /// `m_last_register`; 0 at power-on).
    last_reg: usize,
    /// Resolved per-channel amplitude, updated on attenuation writes.
    volume: [f32; 4],
    /// Live counter reload values, in internal ticks: half the square-wave
    /// period for tones, the inter-shift interval for noise.
    period: [u32; 4],
    /// Down-counters; a step at ≤0 fires the channel and reloads `period`.
    count: [i32; 4],
    /// Tone output flip-flops.
    tone_out: [bool; 3],
    /// Noise LFSR; bit 0 is the noise output.
    lfsr: u32,
    /// Fractional internal ticks carried between [`SN76489A::sample`] calls,
    /// so an arbitrary host sampling cadence stays pitch-exact.
    tick_frac: f64,
    /// Amplitude lookup for attenuation codes 0-15. Skipped: pure
    /// construction-time scratch, rebuilt by [`SN76489A::after_restore`]
    /// via [`build_vol_table`] (`docs/plan-save-states.md`).
    #[serde(skip)]
    vol_table: [f32; 16],
}

impl std::fmt::Debug for SN76489A {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SN76489A")
            .field("regs", &self.regs)
            .field("last_reg", &self.last_reg)
            .field("lfsr", &self.lfsr)
            .finish()
    }
}

impl SN76489A {
    /// A chip clocked by a `crystal_hz` crystal (4 MHz on the GMC).
    ///
    /// Power-on state per MAME `device_start`: all registers 0 — which means
    /// **maximum volume** on every channel (attenuation code 0; MAME's
    /// comment records this as tested on the real SN76489A), tone periods
    /// treated as $400, noise period 0 (shifts every tick until register 6
    /// is written), LFSR seeded with the feedback bit. Real GMC software
    /// initializes the chip immediately, but until it does the chip hums —
    /// exactly like the real cartridge at power-on.
    pub fn new(crystal_hz: f64) -> Self {
        let vol_table = build_vol_table();
        Self {
            tick_hz: crystal_hz / CRYSTAL_CYCLES_PER_TICK,
            regs: [0; 8],
            last_reg: 0,
            volume: [vol_table[0]; 4],
            period: [
                u32::from(PERIOD_ZERO_COUNTS_AS),
                u32::from(PERIOD_ZERO_COUNTS_AS),
                u32::from(PERIOD_ZERO_COUNTS_AS),
                0,
            ],
            count: [0; 4],
            tone_out: [false; 3],
            lfsr: LFSR_FEEDBACK,
            tick_frac: 0.0,
            vol_table,
        }
    }

    /// Restore-time fixup after a snapshot round-trip
    /// (`docs/plan-save-states.md`): rebuilds `vol_table`, the skipped
    /// construction-time lookup table, via the same [`build_vol_table`]
    /// helper [`SN76489A::new`] uses.
    pub fn after_restore(&mut self) {
        self.vol_table = build_vol_table();
    }

    /// A command-byte write (the GMC's `$FF41`). Decode per MAME `write()`:
    ///
    /// - LATCH/DATA (bit 7 set): bits 6-4 select the register and become
    ///   `last_reg`; bits 3-0 merge into the register's low nibble.
    /// - DATA-only (bit 7 clear): tone-period registers take bits 5-0 as the
    ///   period's upper 6 bits (completing the 10-bit value); attenuation and
    ///   noise-control registers just take the low nibble again.
    ///
    /// Every write to the noise-control register — latch or continuation —
    /// resets the LFSR to its seed (MAME does this unconditionally for
    /// non-NCR chips).
    pub fn write(&mut self, data: u8) {
        let low_nibble = u16::from(data & 0x0F);
        let r = if data & LATCH_FLAG != 0 {
            let r = usize::from((data >> 4) & 0x07);
            self.last_reg = r;
            self.regs[r] = (self.regs[r] & 0x3F0) | low_nibble;
            r
        } else {
            let r = self.last_reg;
            if r.is_multiple_of(2) && r != REG_NOISE_CTRL {
                // Tone period: upper 6 of the 10 bits.
                self.regs[r] = (self.regs[r] & 0x00F) | (u16::from(data & 0x3F) << 4);
            } else {
                self.regs[r] = (self.regs[r] & 0x3F0) | low_nibble;
            }
            r
        };
        match r {
            0 | 2 | 4 => {
                let c = r / 2;
                self.period[c] = u32::from(if self.regs[r] == 0 {
                    PERIOD_ZERO_COUNTS_AS
                } else {
                    self.regs[r]
                });
                // A live tone-2 change retunes the noise channel when its
                // rate mirrors tone 2 (MAME duplicates this in case 4).
                if c == 2 && self.regs[REG_NOISE_CTRL] & 0x03 == NOISE_RATE_FOLLOWS_TONE2 {
                    self.period[NOISE_CHANNEL] = self.period[2] << 1;
                }
            }
            1 | 3 | 5 | 7 => {
                self.volume[r / 2] = self.vol_table[usize::from(data & 0x0F)];
            }
            REG_NOISE_CTRL => {
                let rate = self.regs[REG_NOISE_CTRL] & 0x03;
                self.period[NOISE_CHANNEL] = if rate == NOISE_RATE_FOLLOWS_TONE2 {
                    self.period[2] << 1
                } else {
                    // Rates 0-2: shift every 32/64/128 ticks (N/512, N/1024,
                    // N/2048 of the crystal).
                    1 << (5 + rate)
                };
                self.lfsr = LFSR_FEEDBACK;
            }
            _ => unreachable!("register index is 3 bits"),
        }
    }

    /// Advance the chip `dt` seconds and return its mean output level over
    /// that interval, 0.0-1.0. Box-filtering instead of point-sampling: tone
    /// fundamentals reach far above the caller's ~15.7 kHz scanline cadence,
    /// and the mean keeps those from aliasing into junk while leaving the
    /// audible range intact. Fractional ticks carry over, so pitch stays
    /// exact at any calling cadence.
    pub fn sample(&mut self, dt: f64) -> f32 {
        self.tick_frac += dt * self.tick_hz;
        let ticks = self.tick_frac as u32;
        self.tick_frac -= f64::from(ticks);
        if ticks == 0 {
            return self.level();
        }
        let mut acc = 0.0f32;
        for _ in 0..ticks {
            self.tick();
            acc += self.level();
        }
        acc / ticks as f32
    }

    /// One internal tick (crystal/16): step the three tone counters and the
    /// noise counter, toggling flip-flops / shifting the LFSR on expiry
    /// (MAME `sound_stream_update`'s inner loop).
    fn tick(&mut self) {
        for c in 0..3 {
            self.count[c] -= 1;
            if self.count[c] <= 0 {
                self.tone_out[c] = !self.tone_out[c];
                self.count[c] = self.period[c] as i32;
            }
        }
        self.count[NOISE_CHANNEL] -= 1;
        if self.count[NOISE_CHANNEL] <= 0 {
            self.shift_lfsr();
            self.count[NOISE_CHANNEL] = self.period[NOISE_CHANNEL] as i32;
        }
    }

    /// White mode XORs taps $04 and $08 into the feedback bit; periodic mode
    /// holds tap 2 at 0, so only tap 1 feeds back — a single set bit then
    /// circulates over 15 shifts (the classic 1/15-duty "periodic noise").
    fn shift_lfsr(&mut self) {
        let tap1 = self.lfsr & LFSR_TAP1 != 0;
        let tap2 =
            self.lfsr & LFSR_TAP2 != 0 && self.regs[REG_NOISE_CTRL] & NOISE_MODE_WHITE != 0;
        self.lfsr >>= 1;
        if tap1 != tap2 {
            self.lfsr |= LFSR_FEEDBACK;
        }
    }

    /// Instantaneous summed output, 0.0-1.0: each tone contributes its
    /// volume while its flip-flop is high, the noise channel while LFSR
    /// bit 0 is set. Unipolar — the frontend's DC filter removes the offset,
    /// as with the CoCo's own DAC.
    fn level(&self) -> f32 {
        let mut sum = 0.0;
        for c in 0..3 {
            if self.tone_out[c] {
                sum += self.volume[c];
            }
        }
        if self.lfsr & 1 != 0 {
            sum += self.volume[NOISE_CHANNEL];
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4 MHz — the GMC's crystal.
    const CRYSTAL_HZ: f64 = 4_000_000.0;
    /// A `sample()` interval worth exactly one internal tick.
    const ONE_TICK: f64 = CRYSTAL_CYCLES_PER_TICK / CRYSTAL_HZ;

    fn chip() -> SN76489A {
        SN76489A::new(CRYSTAL_HZ)
    }

    /// Silence every channel (attenuation code 15 on registers 1/3/5/7).
    fn mute_all(psg: &mut SN76489A) {
        for cmd in [0x9F, 0xBF, 0xDF, 0xFF] {
            psg.write(cmd);
        }
    }

    #[test]
    fn latch_plus_data_byte_form_the_ten_bit_tone_period() {
        let mut psg = chip();
        // Tone 0 period 0x264: latch low nibble 4, then data byte 0x26.
        psg.write(0x84);
        psg.write(0x26);
        assert_eq!(psg.regs[0], 0x264);
        assert_eq!(psg.period[0], 0x264);
    }

    #[test]
    fn data_only_byte_reuses_the_last_latched_register() {
        let mut psg = chip();
        psg.write(0x84); // latch tone 0, nibble 4
        psg.write(0x26); // data -> tone 0 upper bits
        psg.write(0x01); // still tone 0: replaces upper bits only
        assert_eq!(psg.regs[0], 0x014);
    }

    #[test]
    fn period_zero_counts_as_0x400() {
        let mut psg = chip();
        psg.write(0x80); // tone 0 period = 0
        psg.write(0x00);
        assert_eq!(psg.regs[0], 0);
        assert_eq!(psg.period[0], u32::from(PERIOD_ZERO_COUNTS_AS));
    }

    #[test]
    fn tone_flip_flop_toggles_every_period_ticks() {
        const PERIOD: u16 = 100;
        let mut psg = chip();
        mute_all(&mut psg);
        psg.write(0x90); // tone 0 attenuation 0 (max)
        psg.write(0x80 | (PERIOD & 0x0F) as u8);
        psg.write((PERIOD >> 4) as u8);

        // Tick one at a time (directly, so the count is exact); record the
        // interval between output changes.
        psg.tick();
        let mut last = psg.level();
        let mut transitions = Vec::new();
        for t in 0..2000u32 {
            psg.tick();
            let level = psg.level();
            if level != last {
                transitions.push(t);
                last = level;
            }
        }
        assert!(transitions.len() >= 2, "tone never toggled");
        for pair in transitions.windows(2) {
            assert_eq!(
                pair[1] - pair[0],
                u32::from(PERIOD),
                "half-period must be exactly the period register value in ticks"
            );
        }
    }

    #[test]
    fn attenuation_is_2db_per_step_and_code_15_is_silence() {
        let psg = chip();
        assert_eq!(psg.vol_table[0], CHANNEL_FULL_SCALE as f32);
        assert_eq!(psg.vol_table[15], 0.0);
        for code in 0..14 {
            let ratio = psg.vol_table[code + 1] / psg.vol_table[code];
            assert!(
                (f64::from(ratio) - 1.0 / ATTENUATION_STEP).abs() < 1e-6,
                "step {code} ratio {ratio}"
            );
        }
    }

    #[test]
    fn volume_write_applies_immediately_from_the_data_nibble() {
        let mut psg = chip();
        psg.write(0x95); // tone 0 attenuation 5
        assert_eq!(psg.volume[0], psg.vol_table[5]);
        psg.write(0x03); // DATA-only to the same register
        assert_eq!(psg.volume[0], psg.vol_table[3]);
    }

    #[test]
    fn noise_rate_field_selects_32_64_128_tick_periods() {
        let mut psg = chip();
        for (rate, want) in [(0u8, 32u32), (1, 64), (2, 128)] {
            psg.write(0xE0 | rate);
            assert_eq!(psg.period[NOISE_CHANNEL], want);
        }
    }

    #[test]
    fn noise_rate_3_mirrors_tone_2_doubled_and_tracks_live_retunes() {
        let mut psg = chip();
        psg.write(0xC8); // tone 2 period low nibble 8
        psg.write(0x02); // upper bits -> period 0x028
        psg.write(0xE3); // noise: rate follows tone 2
        assert_eq!(psg.period[NOISE_CHANNEL], 0x28 << 1);
        psg.write(0xC4); // retune tone 2 low nibble -> 0x024
        assert_eq!(psg.period[NOISE_CHANNEL], 0x24 << 1);
    }

    #[test]
    fn any_noise_control_write_reseeds_the_lfsr() {
        let mut psg = chip();
        psg.write(0xE0);
        for _ in 0..100 {
            psg.shift_lfsr();
        }
        assert_ne!(psg.lfsr, LFSR_FEEDBACK);
        psg.write(0xE4); // latch write to reg 6
        assert_eq!(psg.lfsr, LFSR_FEEDBACK);
        for _ in 0..100 {
            psg.shift_lfsr();
        }
        psg.write(0x04); // DATA-only continuation to reg 6 reseeds too
        assert_eq!(psg.lfsr, LFSR_FEEDBACK);
    }

    #[test]
    fn periodic_noise_is_one_set_bit_circulating_over_15_shifts() {
        let mut psg = chip();
        psg.write(0xE0); // periodic (bit 2 clear), reseeds LFSR
        let mut outputs = Vec::new();
        for _ in 0..60 {
            psg.shift_lfsr();
            outputs.push(psg.lfsr & 1);
        }
        let ones: Vec<usize> = (0..outputs.len()).filter(|&i| outputs[i] == 1).collect();
        assert!(ones.len() >= 3, "periodic noise never reached bit 0");
        for pair in ones.windows(2) {
            assert_eq!(pair[1] - pair[0], 15, "periodic noise period must be 15");
        }
    }

    #[test]
    fn white_noise_diverges_from_periodic() {
        let sequence = |mode_cmd: u8| {
            let mut psg = chip();
            psg.write(mode_cmd);
            (0..64)
                .map(|_| {
                    psg.shift_lfsr();
                    psg.lfsr & 1
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(sequence(0xE0), sequence(0xE4));
    }

    #[test]
    fn power_on_state_hums_at_max_volume() {
        // MAME device_start: attenuation 0 (max) everywhere, tone periods
        // $400 — the chip is audible before software touches it.
        let mut psg = chip();
        let mut heard = false;
        for _ in 0..3000 {
            if psg.sample(ONE_TICK) > 0.0 {
                heard = true;
                break;
            }
        }
        assert!(heard);
    }

    #[test]
    fn mean_sampling_of_a_fast_tone_settles_near_half_volume() {
        let mut psg = chip();
        mute_all(&mut psg);
        psg.write(0x90); // tone 0 max volume
        psg.write(0x81); // period 1: toggles every tick
        psg.write(0x00);
        // 1000 ticks per sample: the box filter averages the 125 kHz square
        // wave to ~vol/2 instead of aliasing.
        let level = psg.sample(1000.0 * ONE_TICK);
        let half = psg.vol_table[0] / 2.0;
        assert!(
            (level - half).abs() < 0.01,
            "expected ~{half}, got {level}"
        );
    }
}
