//! Cassette tape deck: PIA1 $FF20/$FF21 tape I/O (CSAVE/CLOAD).
//!
//! The tape is stored as the *decoded* byte stream — the .cas convention:
//! leaders, sync bytes and blocks as plain bytes, "what the BIOS reads and
//! writes", not audio samples. Recording captures the DAC output with
//! cycle-accurate timestamps and demodulates it to bytes; playback
//! synthesizes the squared FSK signal the SALT chip's zero-crossing detector
//! would produce and feeds it to PIA1 PA0 (`cassette-verified-facts`).
//!
//! FSK timing was measured empirically against the stock ROM's CSAVE — the
//! bit-bang code lives in the $A000–$BFFF Color BASIC region that SEB
//! Unravelled II does not cover, so it cannot be derived from the local docs.
//! Measured: one full sine cycle per bit, serialized LSB first; see
//! [`ZERO_BIT_PERIOD`]/[`ONE_BIT_PERIOD`].

use serde::{Deserialize, Serialize};

/// Half-cycle durations, in CPU cycles, of the tape sine the stock ROM
/// writes — measured empirically against `roms/coco3.rom` with
/// `examples/cassette_calibrate.rs` (modal midpoint-crossing spacings). The
/// ROM's waveform is slightly asymmetric (the high half runs shorter than
/// the low half) and playback mirrors it exactly: the ROM's demodulator
/// classifies bits by polling-loop counts of these very widths, and a
/// symmetric wave puts the 0-bit halves on its decision boundary.
///
/// 0 bit: 396 + 418 = 814-cycle full period, ~1100 Hz (nominal "1200 Hz").
const ZERO_BIT_HIGH: u32 = 396;
const ZERO_BIT_LOW: u32 = 418;
/// 1 bit: 207 + 227 = 434-cycle full period, ~2060 Hz (nominal "2400 Hz").
const ONE_BIT_HIGH: u32 = 207;
const ONE_BIT_LOW: u32 = 227;

/// Motor spin-up: cycles after the relay closes before the tape reaches
/// speed and bits start flowing. The ROM pairs every motor-on with a blind
/// ~0.5 s countdown (`LA7D1`: 65536 iterations x 8 cycles, Color BASIC
/// Unravelled) precisely because real mechanisms need this long — without
/// modeling it, the tape rolls during the ROM's blind window and CLOAD's
/// second `CASON` ($A77C) lock-on eats the 128-byte leader before it ever
/// listens (CLOAD cycles the motor off/on between the namefile and data
/// blocks: `LA701`/`LA4D0`).
const MOTOR_SPINUP_CYCLES: u32 = 65536 * 8;

/// Full tone periods, one cycle per bit (measured; the demodulator's unit).
///
/// `pub(crate)`: shared with [`crate::cassette_wav`], which re-synthesizes/
/// decodes these same tones as WAV audio and must use these exact measured
/// values rather than duplicating the magic numbers.
pub(crate) const ZERO_BIT_PERIOD: u32 = ZERO_BIT_HIGH + ZERO_BIT_LOW;
pub(crate) const ONE_BIT_PERIOD: u32 = ONE_BIT_HIGH + ONE_BIT_LOW;

/// Demodulation decision boundary between the two measured periods
/// (midpoint of 455 and 793): a full period at or below this is a 1 bit.
const BIT_PERIOD_THRESHOLD: u64 = (ZERO_BIT_PERIOD as u64 + ONE_BIT_PERIOD as u64) / 2;

/// A period twice the 0-bit's is no tone at all: a discontinuity (motor
/// spin-up glitch, inter-block artifact). The demodulator drops sync and
/// re-hunts for a leader when it sees one.
const PERIOD_BREAK: u64 = 2 * ZERO_BIT_PERIOD as u64;

/// Leader byte: alternating bits used for bit-sync (Service Manual §5.10).
const LEADER: u8 = 0x55;
/// Block sync byte following a leader run (Service Manual §5.10).
///
/// `pub(crate)`: [`crate::cassette_wav::decode_wav`] uses this to pick the
/// more plausible of its two polarity guesses.
pub(crate) const SYNC: u8 = 0x3C;

/// One DAC level change while the motor was on: the new 6-bit level and the
/// motor-on cycle-clock value at the moment it took effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub level: u8,
    pub cycle: u64,
}

/// A virtual tape deck: at most one tape mounted, either being played (the
/// byte stream is fed to PA0 bit by bit while the motor runs) or recorded
/// over (the DAC capture is demodulated into a fresh byte stream when the
/// recording is finalized — see [`Cassette::finalize_recording`]).
#[derive(Default, Serialize, Deserialize)]
pub struct Cassette {
    /// Motor-on cycle clock: advanced by [`Cassette::tick`] only while the
    /// motor relay (PIA1 CA2) is energized — tape position doesn't move
    /// otherwise. Freezing it across motor-off gaps also collapses the
    /// inter-block silences out of the capture.
    clock: u64,
    /// Last DAC level observed, so [`Cassette::record_dac`] only appends on
    /// an actual change.
    last_level: Option<u8>,
    /// Raw record capture: every DAC transition while the motor was on —
    /// the user's own in-flight recording, so this is real unsaved state
    /// and is stored, not skipped.
    capture: Vec<Transition>,
    /// Whether a tape is mounted at all (a blank tape is an empty stream, so
    /// emptiness can't stand in for "no tape").
    mounted: bool,
    /// The mounted tape's decoded byte stream (.cas content). Skipped: a
    /// mounted tape's bytes are media (commercial tapes are copyrighted),
    /// referenced by path+hash rather than embedded in a snapshot; restored
    /// via [`Cassette::reattach_tape`] (`docs/plan-save-states.md`).
    #[serde(skip)]
    tape: Vec<u8>,
    /// Playback position: next byte, next bit (0–7, LSB first), and CPU
    /// cycles already spent inside the current bit's tone cycle.
    pos: usize,
    bit: u8,
    bit_elapsed: u32,
    /// Spin-up countdown: nonzero right after a motor-off→on transition;
    /// the tape holds still until it drains (see [`MOTOR_SPINUP_CYCLES`]).
    spinup_left: u32,
    /// Motor state seen by the previous [`Cassette::tick`], to detect the
    /// off→on transition that (re)arms the spin-up.
    motor_was_on: bool,
    /// A finalized recording has replaced `tape` and hasn't been saved yet.
    dirty: bool,
}

impl Cassette {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount a tape (decoded .cas bytes; empty = blank tape ready to record),
    /// rewound, discarding any unsaved capture.
    pub fn insert_tape(&mut self, bytes: Vec<u8>) {
        *self = Self {
            mounted: true,
            tape: bytes,
            ..Self::default()
        };
    }

    /// Unmount. Callers wanting the recording saved must call
    /// [`Cassette::finalize_recording`] and write [`Cassette::tape_bytes`]
    /// back first (mirrors the floppy eject/write-back split).
    pub fn eject_tape(&mut self) {
        *self = Self::default();
    }

    pub fn has_tape(&self) -> bool {
        self.mounted
    }

    /// Restore-path-only: re-inject a mounted tape's bytes after a snapshot
    /// restore, without resetting the deserialized playback/record state
    /// (`pos`/`bit`/`bit_elapsed`/`capture`/…) the way [`Cassette::insert_tape`]
    /// would (`docs/plan-save-states.md`). `tape` itself is `#[serde(skip)]`
    /// (media bytes are never embedded in a snapshot); everything else on
    /// `self` already came back from the snapshot as-is. Errors (instead of
    /// panicking) if the restored `pos` no longer fits the reattached tape —
    /// the file changed shape since the snapshot was taken — or if `bit`
    /// (an ordinary deserialized field a hand-crafted payload can set to
    /// anything) is out of its `0..8` range: [`Cassette::current_bit_is_one`]
    /// shifts a byte right by `bit` with no bounds check of its own, which
    /// panics on overflow in debug builds and becomes a masked shift (the
    /// amount wrapped to the type's width) in release
    /// (`docs/plan-save-states.md`). Also confirms `pos`/`bit` consistency
    /// exactly at end-of-tape: [`Cassette::tick`] only ever advances `pos`
    /// in the same step that wraps `bit` back to 0, so `pos == tape.len()`
    /// with a nonzero `bit` is itself a sign of a corrupted payload.
    pub fn reattach_tape(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        if self.bit >= 8 {
            return Err(format!(
                "cassette reattach: restored bit index {} is out of range (must be < 8)",
                self.bit
            ));
        }
        if self.pos > bytes.len() {
            return Err(format!(
                "cassette reattach: restored position {} is past the end of the \
                 reattached tape ({} bytes) — the file changed since the snapshot",
                self.pos,
                bytes.len()
            ));
        }
        if self.pos == bytes.len() && self.bit != 0 {
            return Err(format!(
                "cassette reattach: restored position is exactly at the end of the tape \
                 ({} bytes) but bit index is {} (must be 0 at end-of-tape)",
                bytes.len(),
                self.bit
            ));
        }
        self.tape = bytes;
        Ok(())
    }

    /// Tape contents (the .cas file image).
    pub fn tape_bytes(&self) -> &[u8] {
        &self.tape
    }

    /// A finalized recording hasn't been written back to its file yet.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// Playback progress for UI: (position, length) in tape bytes.
    pub fn position(&self) -> (usize, usize) {
        (self.pos, self.tape.len())
    }

    /// The raw DAC capture so far (calibration/diagnostics — see
    /// `examples/cassette_calibrate.rs`).
    pub fn capture(&self) -> &[Transition] {
        &self.capture
    }

    /// True while mounted, not at the end, i.e. moving whenever the motor is.
    pub fn playing(&self) -> bool {
        self.mounted && self.pos < self.tape.len()
    }

    /// Rewind to the start, first folding any pending recording into the
    /// tape so CSAVE → Rewind → CLOAD works without an eject cycle.
    pub fn rewind(&mut self) {
        self.finalize_recording();
        self.pos = 0;
        self.bit = 0;
        self.bit_elapsed = 0;
    }

    /// Advance the motor-on cycle clock and the playback position. Called
    /// once per instruction from `Machine::run_cycles`, alongside
    /// `bus.cart.tick` (same per-instruction cadence as the FD-502
    /// precedent) — per-scanline would be far too coarse against the
    /// ~217-cycle half-periods of the 1-bit tone.
    pub fn tick(&mut self, cycles: u32, motor_on: bool) {
        if motor_on && !self.motor_was_on {
            self.spinup_left = MOTOR_SPINUP_CYCLES;
        }
        self.motor_was_on = motor_on;
        if !motor_on {
            return;
        }
        if self.spinup_left > 0 {
            self.spinup_left = self.spinup_left.saturating_sub(cycles);
            return;
        }
        self.clock += u64::from(cycles);
        if !self.playing() {
            return;
        }
        self.bit_elapsed += cycles;
        while self.playing() {
            let period = self.current_bit_period();
            if self.bit_elapsed < period {
                break;
            }
            self.bit_elapsed -= period;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        if !self.playing() {
            self.bit_elapsed = 0;
        }
    }

    /// The squared tape signal as PA0 sees it: idle high with no tape moving
    /// (or still spinning up), otherwise a square wave — the SALT
    /// zero-crossing detector's rendering of the tape sine — with each bit
    /// cell opening on its LOW half. The ROM's DAC sine table starts rising,
    /// but the line reaching PA0 is inverted somewhere in the record→play
    /// analog path (AC coupling/comparator polarity in the SALT): verified
    /// empirically — the ROM's `CASON` ($A77C) lock never succeeds with
    /// high-first cells and locks reliably with low-first.
    pub fn input_bit(&self) -> bool {
        if !self.motor_was_on || self.spinup_left > 0 || !self.playing() {
            return true;
        }
        let high = if self.current_bit_is_one() {
            ONE_BIT_HIGH
        } else {
            ZERO_BIT_HIGH
        };
        self.bit_elapsed >= high
    }

    fn current_bit_is_one(&self) -> bool {
        self.tape[self.pos] >> self.bit & 1 == 1
    }

    fn current_bit_period(&self) -> u32 {
        if self.current_bit_is_one() {
            ONE_BIT_PERIOD
        } else {
            ZERO_BIT_PERIOD
        }
    }

    /// Sample the cassette-out DAC tap after a PIA1 write. The record output
    /// is a direct unconditional tap of the DAC (not gated by SNDEN/the mux —
    /// `cassette-verified-facts`), so this is called on every PIA1 write, and
    /// only appends when the level actually changed and the motor is on.
    pub fn record_dac(&mut self, level: u8, motor_on: bool) {
        if !motor_on {
            self.last_level = None; // next motor-on write starts a fresh run
            return;
        }
        if self.last_level != Some(level) {
            self.capture.push(Transition {
                level,
                cycle: self.clock,
            });
            self.last_level = Some(level);
        }
    }

    /// Demodulate the DAC capture and, if it contains at least one valid
    /// leader+sync, replace the tape with it (marking it dirty) — CSAVE has
    /// recorded over whatever was mounted. A capture without a sync (stray
    /// DAC noise from sound playback, or nothing at all) is discarded and
    /// the tape kept, so rewinding after a CLOAD never wipes the tape.
    pub fn finalize_recording(&mut self) {
        let decoded = demodulate(&self.capture);
        self.capture.clear();
        self.last_level = None;
        if self.mounted && decoded.contains(&SYNC) {
            self.tape = decoded;
            self.dirty = true;
        }
    }
}

/// Demodulate a DAC transition capture into the decoded byte stream.
///
/// Crossings of the waveform midpoint mark the tone phase; the time between
/// consecutive *rising* crossings is one full tone cycle = one bit
/// ([`BIT_PERIOD_THRESHOLD`] splits 1 from 0) — see [`capture_to_bits`]. Byte
/// alignment is recovered the way the BIOS does it: hunt bit-by-bit for
/// [`LEADER`] bytes then the [`SYNC`], then read the block structure (type,
/// length, payload, checksum, trailer) byte-aligned, and go back to hunting
/// — see [`bits_to_bytes`] — so a glitch between blocks only costs
/// re-syncing on the next leader, exactly like real tape.
pub fn demodulate(capture: &[Transition]) -> Vec<u8> {
    bits_to_bytes(capture_to_bits(capture))
}

/// Crossing-detect a DAC transition capture into a demodulated bit stream:
/// one entry per detected tone cycle, `None` marking a discontinuity (period
/// too long to be a tone — a motor spin-up glitch or inter-block artifact).
fn capture_to_bits(capture: &[Transition]) -> Vec<Option<bool>> {
    let Some(max) = capture.iter().map(|t| t.level).max() else {
        return Vec::new();
    };
    if max == 0 {
        return Vec::new();
    }
    let mid = max / 2;

    let mut bits: Vec<Option<bool>> = Vec::new();
    let mut side = capture[0].level > mid;
    // A capture that starts on the high side starts mid-cycle: count the
    // first period from its first sample, or the opening bit is lost.
    let mut last_rise: Option<u64> = side.then_some(capture[0].cycle);
    let mut last_fall: Option<u64> = None;
    for t in &capture[1..] {
        let new_side = t.level > mid;
        if new_side && !side {
            if let Some(prev) = last_rise {
                let period = t.cycle - prev;
                bits.push(if period > PERIOD_BREAK {
                    None
                } else {
                    Some(period <= BIT_PERIOD_THRESHOLD)
                });
            }
            last_rise = Some(t.cycle);
        } else if !new_side && side {
            last_fall = Some(t.cycle);
        }
        side = new_side;
    }
    // The recording stops with the last tone cycle's closing rise never
    // written (the ROM turns the motor off right after the final byte), so
    // the stream's very last bit dangles. Salvage it from its half-width —
    // without it the tape loses the EOF block's trailer and the ROM's
    // BITIN ($A755) polls forever for the missing edge on playback.
    if let (Some(rise), Some(fall)) = (last_rise, last_fall)
        && fall > rise
        && fall - rise <= PERIOD_BREAK / 2
    {
        bits.push(Some(2 * (fall - rise) <= BIT_PERIOD_THRESHOLD));
    }
    bits
}

/// How far through a locked (byte-aligned) block [`bits_to_bytes`]'s reader is.
enum BlockState {
    /// Bit-level hunt: sliding window looking for LEADER runs, then SYNC.
    Hunt,
    /// Byte-aligned after a sync: `seen` bytes read so far; `total` is
    /// type + length + payload + checksum + trailer, known once the
    /// length byte (the second one) arrives.
    Locked { seen: usize, total: usize },
}

/// Block bytes besides the payload: type, length, checksum, trailer $55.
const BLOCK_OVERHEAD: usize = 4;

/// Recover byte alignment from a demodulated bit stream ([`capture_to_bits`])
/// the way the BIOS does it: hunt bit-by-bit for [`LEADER`] runs then a
/// [`SYNC`], then read one block byte-aligned (type, length, payload,
/// checksum, trailer) before returning to hunting. A `None` bit (a capture
/// discontinuity) drops any in-progress hunt/lock and starts over.
fn bits_to_bytes(bits: Vec<Option<bool>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut state = BlockState::Hunt;
    let mut window: u8 = 0;
    let mut window_bits = 0u32;
    let mut leader_count = 0usize;
    for bit in bits {
        let Some(bit) = bit else {
            state = BlockState::Hunt;
            window = 0;
            window_bits = 0;
            leader_count = 0;
            continue;
        };
        window = window >> 1 | u8::from(bit) << 7; // LSB arrives first
        window_bits += 1;
        match state {
            BlockState::Hunt => {
                if window_bits < 8 {
                    continue;
                }
                if window == LEADER {
                    leader_count += 1;
                    window_bits = 0;
                } else if window == SYNC {
                    out.extend(std::iter::repeat_n(LEADER, leader_count));
                    out.push(SYNC);
                    leader_count = 0;
                    window_bits = 0;
                    state = BlockState::Locked {
                        seen: 0,
                        total: usize::MAX,
                    };
                }
            }
            BlockState::Locked {
                ref mut seen,
                ref mut total,
            } => {
                if window_bits < 8 {
                    continue;
                }
                out.push(window);
                window_bits = 0;
                *seen += 1;
                if *seen == 2 {
                    // `window` is the length byte: the payload size.
                    *total = usize::from(window) + BLOCK_OVERHEAD;
                }
                if *seen >= *total {
                    state = BlockState::Hunt;
                }
            }
        }
    }
    // A trailing leader run with no sync after it (e.g. the stream ended
    // mid-gap) is still tape content.
    out.extend(std::iter::repeat_n(LEADER, leader_count));
    out
}
