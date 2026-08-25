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

mod demodulate;
#[doc(hidden)]
pub mod test_support;

pub use demodulate::demodulate;

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

/// Motor-off duration, while an in-flight recording capture is pending,
/// after which the recording auto-finalizes without requiring an explicit
/// rewind or eject. CSAVE legitimately drops the motor for ~0.5 s between
/// its namefile and data blocks (the ROM's blind `LA7D1` delay — see
/// [`MOTOR_SPINUP_CYCLES`]'s doc), so "recording finished" can only be
/// detected as "motor stayed off much longer than any intra-operation
/// gap"; 2 s clears that with wide margin while still saving promptly.
const RECORD_IDLE_FINALIZE_SECONDS: f64 = 2.0;

/// [`RECORD_IDLE_FINALIZE_SECONDS`] expressed in CPU cycles at the real
/// clock ([`crate::CPU_HZ`]). `pub` (mirrors
/// [`crate::drivewire::TRANSACTION_TIMEOUT_CYCLES`]'s pattern) so the
/// workspace's test suites can tick past the threshold without re-deriving
/// or duplicating the cycle count.
pub const RECORD_IDLE_FINALIZE_CYCLES: u64 = (crate::CPU_HZ * RECORD_IDLE_FINALIZE_SECONDS) as u64;

/// Full tone periods, one cycle per bit (measured; the demodulator's unit).
///
/// `pub(crate)`: shared with [`crate::cassette_wav`], which re-synthesizes/
/// decodes these same tones as WAV audio and must use these exact measured
/// values rather than duplicating the magic numbers.
pub(crate) const ZERO_BIT_PERIOD: u32 = ZERO_BIT_HIGH + ZERO_BIT_LOW;
pub(crate) const ONE_BIT_PERIOD: u32 = ONE_BIT_HIGH + ONE_BIT_LOW;

/// Leader byte: alternating bits used for bit-sync (Service Manual §5.10).
const LEADER: u8 = 0x55;
/// Block sync byte following a leader run (Service Manual §5.10).
///
/// `pub(crate)`: [`crate::cassette_wav::decode_wav`] uses this to pick the
/// more plausible of its two polarity guesses.
pub(crate) const SYNC: u8 = 0x3C;

/// The 6-bit cassette DAC's full scale (levels 0–63).
const DAC_FULL_SCALE: u8 = 63;

/// DAC midpoint the live record counter judges crossings against: half of
/// [`DAC_FULL_SCALE`]. [`demodulate`] re-derives its midpoint from the
/// finished capture's own maximum; the live counter has to classify each
/// sample as it arrives, so it uses the nominal midpoint — the stock ROM's
/// CSAVE sine swings the full scale, so both land on the same crossings.
const DAC_LIVE_MIDPOINT: u8 = DAC_FULL_SCALE / 2;

/// One DAC level change while the motor was on: the new 6-bit level and the
/// motor-on cycle-clock value at the moment it took effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub level: u8,
    pub cycle: u64,
}

/// A virtual tape deck: at most one tape mounted, either being played (the
/// byte stream is fed to PA0 bit by bit while the motor runs) or recorded
/// over — like a real deck, recording splices the demodulated capture into
/// the tape at the head position rather than replacing the whole tape (see
/// [`Cassette::finalize_recording`]).
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
    /// Live record counter: rising [`DAC_LIVE_MIDPOINT`] crossings seen in
    /// the in-flight capture — approximately (±1) one tone cycle per bit: a
    /// still-in-progress burst registers one more rising crossing than it
    /// has completed periods, so the live byte estimate can sit one bit
    /// high. Good enough for [`Cassette::position`] to move during a
    /// recording without demodulating the capture every frame; the accurate
    /// count only comes from [`Cassette::finalize_recording`]'s own decode.
    /// `#[serde(default)]` (snapshot evolution rule 2, [`crate::snapshot`]):
    /// a pre-field snapshot restores with the counter at 0, exactly the old
    /// behaviour.
    #[serde(default)]
    record_bits: u64,
    /// Whether the last captured level sat above [`DAC_LIVE_MIDPOINT`], for
    /// the counter's crossing detection (`#[serde(default)]` as above).
    #[serde(default)]
    record_high: bool,
    /// The splice point: `pos` at the moment the in-flight capture's first
    /// transition was recorded (see [`Cassette::record_dac`]). Real tape
    /// records starting at the head's current position rather than
    /// replacing the whole reel, so [`Cassette::finalize_recording`] grafts
    /// the demodulated capture into `tape` at this offset instead of
    /// overwriting it outright. `#[serde(default)]` (snapshot evolution rule
    /// 2, [`crate::snapshot`]): a pre-field snapshot restores with the
    /// anchor at 0, which reproduces the old whole-tape-replace behaviour
    /// (truncating to 0 and extending is exactly a replace).
    #[serde(default)]
    record_anchor: usize,
    /// Whether a tape is mounted at all (a blank tape is an empty stream, so
    /// emptiness can't stand in for "no tape").
    mounted: bool,
    /// The mounted tape's decoded byte stream (.cas content). Skipped: a
    /// mounted tape's bytes are media (commercial tapes are copyrighted),
    /// referenced by path+hash rather than embedded in a snapshot; restored
    /// via [`Cassette::reattach_tape`].
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
    /// A finalized recording has spliced into `tape` and hasn't been saved
    /// yet.
    dirty: bool,
    /// Motor-off cycles accumulated while a capture is in flight, counting
    /// towards [`RECORD_IDLE_FINALIZE_CYCLES`] auto-finalizing the recording.
    /// Reset whenever the motor is on, and in [`Cassette::finalize_recording`]
    /// alongside the other record-state resets. `#[serde(default)]` (snapshot
    /// evolution rule 2, [`crate::snapshot`]): a pre-field snapshot restores
    /// with the counter at 0 — worst case a snapshot taken mid-gap waits the
    /// full idle window again before auto-finalizing, not a correctness issue.
    #[serde(default)]
    idle_cycles: u64,
    /// Set when [`Cassette::finalize_recording`] actually spliced new content
    /// into the tape (never on a discarded sync-less capture) — the
    /// frontend's cue to save the tape back to disk without waiting for an
    /// eject/quit boundary; consumed via [`Cassette::take_recording_landed`].
    /// `#[serde(default)]` (snapshot evolution rule 2, [`crate::snapshot`]): a
    /// pre-field snapshot restores with the flag clear, exactly the old (no
    /// auto-save) behaviour.
    #[serde(default)]
    recording_landed: bool,
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

    /// Unmount. To save an in-flight recording first, call
    /// [`Cassette::finalize_recording`] and write [`Cassette::tape_bytes`] back.
    pub fn eject_tape(&mut self) {
        *self = Self::default();
    }

    pub fn has_tape(&self) -> bool {
        self.mounted
    }

    /// Re-inject a mounted tape's bytes after a snapshot restore, without resetting other
    /// deserialized state. Errors instead of panicking if the restored `pos`/`bit` no longer fit.
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

    /// Consume the "a recording just landed" event (true once per landed
    /// finalize, see [`Cassette::finalize_recording`]) so a failed disk
    /// write doesn't retry every frame.
    pub fn take_recording_landed(&mut self) -> bool {
        std::mem::take(&mut self.recording_landed)
    }

    /// Tape position for UI: (position, length) in bytes. During playback it's the read head;
    /// while recording, the live estimate from [`Cassette::record_anchor`] plus counted bits.
    pub fn position(&self) -> (usize, usize) {
        if !self.mounted || self.capture.is_empty() {
            return (self.pos, self.tape.len());
        }
        let recorded_bytes = usize::try_from(self.record_bits / 8).unwrap_or(usize::MAX);
        let recorded = self.record_anchor.saturating_add(recorded_bytes);
        (recorded, recorded.max(self.tape.len()))
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

    /// Rewind to the start: shorthand for [`Cassette::seek`]`(0)`.
    pub fn rewind(&mut self) {
        self.seek(0);
    }

    /// Move the read/record head to a byte position (UI "seek to byte"),
    /// first finalizing any pending recording; clamps to the tape's end.
    pub fn seek(&mut self, pos: usize) {
        self.finalize_recording();
        self.pos = pos.min(self.tape.len());
        self.bit = 0;
        self.bit_elapsed = 0;
    }

    /// Advance the motor-on cycle clock and playback position once per CPU unit. While the motor
    /// is off, track idle time toward auto-finalizing a pending recording.
    pub fn tick(&mut self, cycles: u32, motor_on: bool) {
        if motor_on && !self.motor_was_on {
            self.spinup_left = MOTOR_SPINUP_CYCLES;
        }
        self.motor_was_on = motor_on;
        if !motor_on {
            if !self.capture.is_empty() {
                self.idle_cycles += u64::from(cycles);
                if self.idle_cycles >= RECORD_IDLE_FINALIZE_CYCLES {
                    self.finalize_recording();
                }
            }
            return;
        }
        self.idle_cycles = 0;
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

    /// The squared tape signal as PA0 sees it: idle high, else a square wave with each
    /// bit cell opening LOW (SALT's inverted rendering, verified via `CASON` lock behavior).
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

    /// Sample the cassette-out DAC tap after a PIA1 Port A output/DDR write;
    /// only appends when the level actually changed and the motor is on.
    pub fn record_dac(&mut self, level: u8, motor_on: bool) {
        if !motor_on {
            self.last_level = None; // next motor-on write starts a fresh run
            return;
        }
        if self.last_level != Some(level) {
            if self.capture.is_empty() {
                // First transition: anchor the splice at the head's current position.
                self.record_anchor = self.pos;
            }
            self.capture.push(Transition {
                level,
                cycle: self.clock,
            });
            self.last_level = Some(level);
            // record_bits counts one rising midpoint crossing per tone cycle.
            let high = level > DAC_LIVE_MIDPOINT;
            if high && !self.record_high {
                self.record_bits += 1;
            }
            self.record_high = high;
        }
    }

    /// Demodulate the DAC capture and, if it contains a valid leader+sync, splice it into the
    /// tape at [`Cassette::record_anchor`]; a sync-less capture is discarded, tape untouched.
    pub fn finalize_recording(&mut self) {
        let decoded = demodulate(&self.capture);
        // Captured before the reset below zeroes the field.
        let anchor = self.record_anchor;
        self.capture.clear();
        self.last_level = None;
        self.record_bits = 0;
        self.record_high = false;
        self.record_anchor = 0;
        self.idle_cycles = 0;
        if self.mounted && decoded.contains(&SYNC) {
            self.tape.truncate(anchor);
            self.tape.extend(decoded);
            self.pos = self.tape.len();
            self.bit = 0;
            self.bit_elapsed = 0;
            self.dirty = true;
            self.recording_landed = true;
        }
    }
}
