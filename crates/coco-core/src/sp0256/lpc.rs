//! The SP0256's 12-pole LPC synthesis filter: a periodic-impulse or noise
//! excitation through six cascaded second-order stages, driven by the 16
//! registers the microsequencer loads (MAME `sp0256.cpp` `lpc12_t`).

use serde::{Deserialize, Serialize};

/// Excitation period substituted for a PAUSE opcode (MAME `PER_PAUSE`).
pub(super) const PER_PAUSE: u8 = 64;
/// Excitation period while producing noise, i.e. when register period is 0
/// (MAME `PER_NOISE`).
const PER_NOISE: i32 = 64;
/// Second-order stages in the cascade.
const STAGES: usize = 6;
/// Encoded register-set size.
pub(super) const REGISTER_COUNT: usize = 16;
/// 15-bit LFSR feedback taps for the noise source (MAME `0x4001`).
const NOISE_TAPS: u32 = 0x4001;
/// Non-zero LFSR seed (MAME `m_filt.rng = 1`).
const NOISE_SEED: u32 = 1;
/// Output limiter range before the final `<< 2` (MAME `HIGH_QUALITY` `limit`).
const LIMIT_MIN: i16 = -8192;
const LIMIT_MAX: i16 = 8191;
/// Final output scaling: limited 14-bit sample to 16-bit (MAME `limit(samp) << 2`).
const OUTPUT_SHIFT: u32 = 2;
/// Amplitude register: bits 0-4 mantissa, bits 5-7 exponent.
const AMP_MANTISSA_MASK: u8 = 0x1F;
const AMP_EXPONENT_SHIFT: u8 = 5;

/// Encoded register indices (MAME `enum { AM = 0, PR, B0, F0, ... IA, IP }`).
pub(super) mod reg {
    pub const AMPLITUDE: usize = 0;
    pub const PERIOD: usize = 1;
    pub const B0: usize = 2;
    pub const F0: usize = 3;
    pub const B1: usize = 4;
    pub const F1: usize = 5;
    pub const B2: usize = 6;
    pub const F2: usize = 7;
    pub const B3: usize = 8;
    pub const F3: usize = 9;
    pub const B4: usize = 10;
    pub const F4: usize = 11;
    pub const B5: usize = 12;
    pub const F5: usize = 13;
    pub const AMPLITUDE_INTERP: usize = 14;
    pub const PERIOD_INTERP: usize = 15;
}

/// Coefficient quantization table (MAME `qtbl`, from the SP0250 data sheet).
#[rustfmt::skip]
const QTBL: [i16; 128] = [
    0,      9,      17,     25,     33,     41,     49,     57,
    65,     73,     81,     89,     97,     105,    113,    121,
    129,    137,    145,    153,    161,    169,    177,    185,
    193,    201,    209,    217,    225,    233,    241,    249,
    257,    265,    273,    281,    289,    297,    301,    305,
    309,    313,    317,    321,    325,    329,    333,    337,
    341,    345,    349,    353,    357,    361,    365,    369,
    373,    377,    381,    385,    389,    393,    397,    401,
    405,    409,    413,    417,    421,    425,    427,    429,
    431,    433,    435,    437,    439,    441,    443,    445,
    447,    449,    451,    453,    455,    457,    459,    461,
    463,    465,    467,    469,    471,    473,    475,    477,
    479,    481,    482,    483,    484,    485,    486,    487,
    488,    489,    490,    491,    492,    493,    494,    495,
    496,    497,    498,    499,    500,    501,    502,    503,
    504,    505,    506,    507,    508,    509,    510,    511,
];

/// Filter state (MAME `lpc12_t`). Arithmetic mirrors the C exactly: the
/// sample accumulator is a wrapping 16-bit value, coefficients are scaled
/// by `>> 9` (B) and `>> 8` (F) per stage.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Lpc12 {
    /// Repeat counter: excitation periods left before the sequencer runs
    /// again. `<= 0` means "fetch the next instruction".
    pub rpt: i32,
    /// Period down-counter.
    cnt: i32,
    /// Excitation period (0 = noise).
    per: u32,
    /// Noise LFSR.
    rng: u32,
    /// Decoded impulse amplitude.
    amp: i32,
    f_coef: [i16; STAGES],
    b_coef: [i16; STAGES],
    /// Per-stage delay line.
    z_data: [[i16; 2]; STAGES],
    /// The encoded register set the sequencer writes.
    pub r: [u8; REGISTER_COUNT],
    /// Whether the interpolation registers are non-zero.
    interp: bool,
}

impl Default for Lpc12 {
    /// MAME `device_reset`: zeroed, then `rpt = -1`, `rng = 1`.
    fn default() -> Self {
        Self {
            rpt: -1,
            cnt: 0,
            per: 0,
            rng: NOISE_SEED,
            amp: 0,
            f_coef: [0; STAGES],
            b_coef: [0; STAGES],
            z_data: [[0; 2]; STAGES],
            r: [0; REGISTER_COUNT],
            interp: false,
        }
    }
}

/// `(mantissa) << exponent` (MAME `amp = (r[0] & 0x1F) << ((r[0] & 0xE0) >> 5)`).
fn decode_amplitude(r0: u8) -> i32 {
    i32::from(r0 & AMP_MANTISSA_MASK) << (r0 >> AMP_EXPONENT_SHIFT)
}

/// Sign-magnitude-ish coefficient decode (MAME `IQ`): bit 7 set selects the
/// positive table entry at the two's-complement negation, else the negated
/// entry.
pub(super) fn inverse_quantize(x: u8) -> i16 {
    if x & 0x80 != 0 {
        QTBL[usize::from(x.wrapping_neg() & 0x7F)]
    } else {
        -QTBL[usize::from(x)]
    }
}

impl Lpc12 {
    /// Decode the register set into amplitude, period, and coefficients, and
    /// force an immediate first impulse (MAME `lpc12_t::regdec`).
    pub fn regdec(&mut self) {
        self.amp = decode_amplitude(self.r[reg::AMPLITUDE]);
        self.cnt = 0;
        self.per = u32::from(self.r[reg::PERIOD]);
        for stage in 0..STAGES {
            self.b_coef[stage] = inverse_quantize(self.r[reg::B0 + 2 * stage]);
            self.f_coef[stage] = inverse_quantize(self.r[reg::F0 + 2 * stage]);
        }
        self.interp = self.r[reg::AMPLITUDE_INTERP] != 0 || self.r[reg::PERIOD_INTERP] != 0;
    }

    /// Advance the excitation by one sample tick. Returns `None` when the
    /// repeat counter expired on this tick — the sequencer must run before
    /// the next sample, and no output is produced for this tick (MAME
    /// `lpc12_t::update` breaks before the filter stage).
    pub fn update_one(&mut self) -> Option<i16> {
        let mut do_int = false;
        let mut samp: i16;
        if self.per != 0 {
            if self.cnt <= 0 {
                self.cnt += self.per as i32;
                samp = self.amp as i16;
                self.rpt -= 1;
                do_int = self.interp;
                self.z_data = [[0; 2]; STAGES];
            } else {
                samp = 0;
                self.cnt -= 1;
            }
        } else {
            self.cnt -= 1;
            if self.cnt <= 0 {
                do_int = self.interp;
                self.cnt = PER_NOISE;
                self.rpt -= 1;
                self.z_data = [[0; 2]; STAGES];
            }
            let bit = self.rng & 1 != 0;
            self.rng = (self.rng >> 1) ^ if bit { NOISE_TAPS } else { 0 };
            samp = if bit {
                self.amp as i16
            } else {
                (self.amp as i16).wrapping_neg()
            };
        }

        if do_int {
            self.r[reg::AMPLITUDE] =
                self.r[reg::AMPLITUDE].wrapping_add(self.r[reg::AMPLITUDE_INTERP]);
            self.r[reg::PERIOD] = self.r[reg::PERIOD].wrapping_add(self.r[reg::PERIOD_INTERP]);
            self.amp = decode_amplitude(self.r[reg::AMPLITUDE]);
            self.per = u32::from(self.r[reg::PERIOD]);
        }

        if self.rpt <= 0 {
            return None;
        }

        for stage in 0..STAGES {
            let [z0, z1] = self.z_data[stage];
            let acc = i32::from(samp)
                + ((i32::from(self.b_coef[stage]) * i32::from(z1)) >> 9)
                + ((i32::from(self.f_coef[stage]) * i32::from(z0)) >> 8);
            samp = acc as i16;
            self.z_data[stage] = [samp, z0];
        }
        Some(samp.clamp(LIMIT_MIN, LIMIT_MAX) << OUTPUT_SHIFT)
    }
}
