//! Sequential sound-data playback engine: parses buffer-RAM event groups
//! ([`group`]) and schedules their durations ([`timing`]) against the AY.
//! See [`super::SoundSpeechCartridge::advance_engine`] for the top-level per-group dispatch.

use serde::{Deserialize, Serialize};

use crate::ay8913::{mixer, reg as ay_reg};

use super::SoundSpeechCartridge;
use super::protocol::terminator;

/// Sound-data event group bit layout. Every group's first byte: bits 7-5 =
/// 3-bit opcode, bit 4 = M (envelope-mode flag; unused for envelope's own
/// first byte, where bits 3-0 are the shape instead), bits 3-0 = amplitude
/// (tone/noise) or envelope shape bits S3-S0.
pub mod group {
    /// Tone channel A/B/C opcodes (bits 7-5 of a tone group's first byte).
    pub const TONE_A: u8 = 0b000;
    pub const TONE_B: u8 = 0b001;
    pub const TONE_C: u8 = 0b010;
    /// Envelope opcode, "low" encoding (bits 7-5 = `0b011`).
    pub const ENVELOPE_LOW: u8 = 0b011;
    /// Noise channel A/B/C opcodes (bits 7-5 of a noise group's first byte).
    pub const NOISE_A: u8 = 0b100;
    pub const NOISE_B: u8 = 0b101;
    pub const NOISE_C: u8 = 0b110;
    /// Envelope opcode, "high" encoding (bits 7-5 = `0b111`).
    pub const ENVELOPE_HIGH: u8 = 0b111;

    /// Right-shift to isolate the 3-bit opcode from a group's first byte.
    pub const OPCODE_SHIFT: u8 = 5;
    /// Mask applied after [`OPCODE_SHIFT`] (3 bits: 0-7).
    pub const OPCODE_MASK: u8 = 0b111;
    /// Bit 4 of a tone/noise group's first byte: envelope-mode flag,
    /// matching the AY's own R8/R9/R10 bit 4.
    pub const M_FLAG: u8 = 0x10;
    /// Bits 3-0 of a tone/noise group's first byte: fixed amplitude level.
    pub const AMP_MASK: u8 = 0x0F;
    /// Bits 3-0 of an envelope group's first byte: shape bits S3-S0.
    pub const SHAPE_MASK: u8 = 0x0F;
    /// Bits 3-0 of a tone group's second byte: coarse tone period (the
    /// AY's own tone-coarse registers are 4 bits wide in silicon).
    pub const TONE_COARSE_MASK: u8 = 0x0F;
    /// Bit 7 of a noise group's second byte: "R" reuse-previous-amplitude
    /// flag (manual: "the amplitude of the preceding data group is used,
    /// and the amplitude bits in the first byte are ignored").
    pub const NOISE_REUSE_FLAG: u8 = 0x80;
    /// Bits 4-0 of a noise group's second byte: 5-bit noise period.
    pub const NOISE_PERIOD_MASK: u8 = 0x1F;

    /// Byte length of a group given its 3-bit opcode: 3 for noise groups, 4
    /// for every other (tone: `[amp, coarse, fine, duration]`; envelope:
    /// `[shape, coarse, fine, duration]`).
    pub fn len(opcode: u8) -> usize {
        match opcode {
            NOISE_A | NOISE_B | NOISE_C => 3,
            _ => 4,
        }
    }
}

/// Synthetic scaling from a sound-data event's `(duration_byte, timer_base)`
/// pair to real elapsed E-clock cycles. NOT A HARDWARE FACT: the manual
/// documents only that 0=shortest and 255=longest for both values
/// independently, with no formula, units, or worked example anywhere in its
/// text. This constant is invented, tuned only to produce audible,
/// plausible note/effect durations, exactly like `BUSY_HOLD_CYCLES`.
pub mod timing {
    /// `duration_cycles = duration_byte * (timer_base + 1) * CYCLES_PER_DURATION_UNIT`.
    pub const CYCLES_PER_DURATION_UNIT: u32 = 400;
    /// No documented power-on/reset default for the timer-base register.
    /// Chosen as a mid-range value so software that never sends `$8F` still
    /// gets audible, non-instant durations. Not a hardware fact.
    pub const DEFAULT_TIMER_BASE: u8 = 32;

    pub fn duration_cycles(duration_byte: u8, timer_base: u8) -> u32 {
        u32::from(duration_byte) * (u32::from(timer_base) + 1) * CYCLES_PER_DURATION_UNIT
    }
}

/// Sequential sound-data playback engine: a single active cursor through one
/// linear byte stream (buffer RAM), one group "playing" (gating the next
/// group's processing) for its scheduled duration at a time. A new
/// execute-sound-data command replaces whatever stream was previously
/// running — there is no queueing or concurrent-channel scheduling here
/// (real hardware achieves simultaneous multi-channel playback via the
/// separate, un-timed register-string LOAD/EXECUTE mechanism instead).
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct Engine {
    /// Whether the engine is currently advancing through a stream. Cleared
    /// at end-of-stream (terminator, incomplete trailing group, or capacity
    /// exhaustion) or by an explicit stop command — the engine simply stops
    /// advancing; it does NOT silence the AY on its own (see
    /// [`SoundSpeechCartridge::advance_engine`]).
    pub(super) active: bool,
    /// Next RAM offset to parse a group's opcode byte from.
    pub(super) cursor: usize,
    /// One past the highest offset this stream may read from (consecutive:
    /// [`super::protocol::ram::SIZE`]; individual: the end of the target
    /// buffer).
    pub(super) cap: usize,
    /// Tracks the most recent group's raw 4-bit amplitude field, for noise
    /// groups' "R" reuse flag. Reset to 0 at the start of every
    /// execute-sound-data command.
    last_amplitude_nibble: u8,
    /// E-clock cycles remaining before the current group's duration elapses
    /// and [`SoundSpeechCartridge::advance_engine`] runs again.
    duration_countdown: u32,
}

impl SoundSpeechCartridge {
    /// `$00`/`$CF` "stop all sound": halts the engine and zeroes all three AY
    /// channel volumes — a true off, unlike natural end-of-stream. Identical
    /// in this implementation for both commands.
    pub(super) fn stop_all_sound(&mut self) {
        self.engine = Engine::default();
        self.ay_write(ay_reg::VOL_A, 0);
        self.ay_write(ay_reg::VOL_B, 0);
        self.ay_write(ay_reg::VOL_C, 0);
    }

    /// Starts a sound-data EXECUTE stream: resets the engine to the given
    /// window and synchronously parses/programs the first group.
    pub(super) fn start_sound_execute(&mut self, start: usize, cap: usize) {
        self.engine = Engine {
            active: true,
            cursor: start,
            cap,
            last_amplitude_nibble: 0,
            duration_countdown: 0,
        };
        self.advance_engine();
    }

    /// Parses and programs exactly one group at [`Engine::cursor`], then
    /// schedules its duration. End-of-stream (terminator, or the next group
    /// doesn't fit) clears [`Engine::active`] without silencing the AY —
    /// the last group's registers stay set until overwritten.
    pub(super) fn advance_engine(&mut self) {
        if !self.engine.active {
            return;
        }
        let cap = self.engine.cap;
        let cursor = self.engine.cursor;
        if cursor >= cap {
            self.engine.active = false;
            return;
        }
        let byte0 = self.ram[cursor];
        if byte0 == terminator::SOUND {
            self.engine.active = false;
            return;
        }
        let opcode = (byte0 >> group::OPCODE_SHIFT) & group::OPCODE_MASK;
        let group_len = group::len(opcode);
        if cursor + group_len > cap {
            // Incomplete trailing group: never parsed or played.
            self.engine.active = false;
            return;
        }
        match opcode {
            group::TONE_A | group::TONE_B | group::TONE_C => {
                self.process_tone_group(cursor, opcode)
            }
            group::NOISE_A | group::NOISE_B | group::NOISE_C => {
                self.process_noise_group(cursor, opcode)
            }
            group::ENVELOPE_LOW | group::ENVELOPE_HIGH => {
                self.process_standalone_envelope_group(cursor)
            }
            _ => unreachable!("opcode is masked to 3 bits (0-7); all 8 values are handled above"),
        }
    }

    /// Programs a tone group (4 bytes: `[op|M|amp, coarse, fine, duration]`)
    /// at `at`, chains a following M=1 envelope group if present, and
    /// schedules the duration.
    fn process_tone_group(&mut self, at: usize, opcode: u8) {
        let byte0 = self.ram[at];
        let m_flag = byte0 & group::M_FLAG != 0;
        let amp = byte0 & group::AMP_MASK;
        let coarse = self.ram[at + 1] & group::TONE_COARSE_MASK;
        let fine = self.ram[at + 2];
        let own_duration = self.ram[at + 3];

        let ch = match opcode {
            group::TONE_A => 0,
            group::TONE_B => 1,
            group::TONE_C => 2,
            _ => unreachable!("caller only dispatches TONE_A/B/C here"),
        };
        self.ay_write(tone_coarse_reg(ch), coarse);
        self.ay_write(tone_fine_reg(ch), fine);
        self.ay_write(vol_reg(ch), amp | if m_flag { group::M_FLAG } else { 0 });
        self.set_mixer_channel(ch, true, false);
        self.engine.last_amplitude_nibble = amp;

        let mut cursor = at + 4;
        let mut duration = own_duration;
        if m_flag {
            let cap = self.engine.cap;
            if let Some((env_duration, env_len)) = self.peek_chained_envelope(cursor, cap) {
                duration = env_duration;
                cursor += env_len;
            }
        }
        self.engine.cursor = cursor;
        self.engine.duration_countdown = timing::duration_cycles(duration, self.timer_base);
    }

    /// Programs a noise group (3 bytes: `[op|M|amp, R|period, duration]`) at
    /// `at`, applying the "R" reuse-amplitude flag, chains a following M=1
    /// envelope group if present, and schedules the duration.
    fn process_noise_group(&mut self, at: usize, opcode: u8) {
        let byte0 = self.ram[at];
        let m_flag = byte0 & group::M_FLAG != 0;
        let raw_amp = byte0 & group::AMP_MASK;
        let byte1 = self.ram[at + 1];
        let reuse = byte1 & group::NOISE_REUSE_FLAG != 0;
        let period = byte1 & group::NOISE_PERIOD_MASK;
        let own_duration = self.ram[at + 2];

        let ch = match opcode {
            group::NOISE_A => 0,
            group::NOISE_B => 1,
            group::NOISE_C => 2,
            _ => unreachable!("caller only dispatches NOISE_A/B/C here"),
        };
        let amp = if reuse {
            self.engine.last_amplitude_nibble
        } else {
            raw_amp
        };

        self.ay_write(ay_reg::NOISE_PERIOD, period);
        self.ay_write(vol_reg(ch), amp | if m_flag { group::M_FLAG } else { 0 });
        self.set_mixer_channel(ch, false, true);
        if !reuse {
            self.engine.last_amplitude_nibble = raw_amp;
        }

        let mut cursor = at + 3;
        let mut duration = own_duration;
        if m_flag {
            let cap = self.engine.cap;
            if let Some((env_duration, env_len)) = self.peek_chained_envelope(cursor, cap) {
                duration = env_duration;
                cursor += env_len;
            }
        }
        self.engine.cursor = cursor;
        self.engine.duration_countdown = timing::duration_cycles(duration, self.timer_base);
    }

    /// Defensive fallback for a standalone envelope group not preceded by an
    /// M=1 tone/noise group (undocumented by the manual) — programmed as its
    /// own event so a malformed stream can't desync or infinite-loop the engine.
    fn process_standalone_envelope_group(&mut self, at: usize) {
        let cap = self.engine.cap;
        let (duration, len) = self.peek_chained_envelope(at, cap).expect(
            "advance_engine already verified an envelope opcode with a full group available",
        );
        self.engine.cursor = at + len;
        self.engine.duration_countdown = timing::duration_cycles(duration, self.timer_base);
    }

    /// If a well-formed envelope group (4 bytes) starts at `after` and fits
    /// within `cap`, programs its AY registers and returns
    /// `(duration_byte, 4)`; returns `None` without touching the AY otherwise.
    fn peek_chained_envelope(&mut self, after: usize, cap: usize) -> Option<(u8, usize)> {
        const ENVELOPE_GROUP_LEN: usize = 4;
        if after + ENVELOPE_GROUP_LEN > cap {
            return None;
        }
        let byte0 = self.ram[after];
        let opcode = (byte0 >> group::OPCODE_SHIFT) & group::OPCODE_MASK;
        if opcode != group::ENVELOPE_LOW && opcode != group::ENVELOPE_HIGH {
            return None;
        }
        let shape = byte0 & group::SHAPE_MASK;
        let coarse = self.ram[after + 1];
        let fine = self.ram[after + 2];
        let duration = self.ram[after + 3];
        self.ay_write(ay_reg::ENV_SHAPE, shape);
        self.ay_write(ay_reg::ENV_COARSE, coarse);
        self.ay_write(ay_reg::ENV_FINE, fine);
        Some((duration, ENVELOPE_GROUP_LEN))
    }

    /// Read-modify-write R7 so channel `ch`'s tone/noise generators are
    /// enabled/disabled, without disturbing the other channels. Judgment
    /// call (not manual-specified): auto-enables the mixer so streams that
    /// never poke R7 via a register-string command still play audibly.
    fn set_mixer_channel(&mut self, ch: usize, tone_enabled: bool, noise_enabled: bool) {
        let mut mixer_val = self.ay_read(ay_reg::MIXER);
        let tone_bit = 1 << (mixer::TONE_DISABLE_SHIFT + ch as u8);
        let noise_bit = 1 << (mixer::NOISE_DISABLE_SHIFT + ch as u8);
        // Mixer bits are active-low: clear to enable, set to disable.
        mixer_val = if tone_enabled {
            mixer_val & !tone_bit
        } else {
            mixer_val | tone_bit
        };
        mixer_val = if noise_enabled {
            mixer_val & !noise_bit
        } else {
            mixer_val | noise_bit
        };
        self.ay_write(ay_reg::MIXER, mixer_val);
    }

    /// Advances the sound-data engine by `cycles` E-clock cycles. If the
    /// current group's duration has elapsed, [`SoundSpeechCartridge::advance_engine`]
    /// runs immediately — no remainder carries into the next event's countdown.
    pub(super) fn tick_engine(&mut self, cycles: u32) {
        if !self.engine.active {
            return;
        }
        if cycles >= self.engine.duration_countdown {
            self.advance_engine();
        } else {
            self.engine.duration_countdown -= cycles;
        }
    }
}

/// Tone channel `ch`'s (0=A, 1=B, 2=C) coarse-period AY register.
fn tone_coarse_reg(ch: usize) -> u8 {
    ay_reg::TONE_A_COARSE + (ch as u8) * 2
}

/// Tone channel `ch`'s (0=A, 1=B, 2=C) fine-period AY register.
fn tone_fine_reg(ch: usize) -> u8 {
    ay_reg::TONE_A_FINE + (ch as u8) * 2
}

/// Channel `ch`'s (0=A, 1=B, 2=C) volume AY register.
fn vol_reg(ch: usize) -> u8 {
    ay_reg::VOL_A + ch as u8
}
