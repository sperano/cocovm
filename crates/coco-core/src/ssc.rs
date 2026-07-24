//! Tandy Sound/Speech Cartridge (SSC, 26-3144): the `$FF7D`/`$FF7E` bus
//! handshake shell, an AY-3-8913 PSG mixed into the machine's audio output,
//! and (as of this module) a host-byte protocol interpreter for the SOUND
//! half of that protocol.
//!
//! **Not modelled** (see `docs/ssc-spec.md` "Deferred Tier 3"): the TMS7040
//! CPU that actually runs the cartridge's firmware and the SP0256-AL2
//! speech synthesizer chip. What *is* modelled is the byte-stream protocol a
//! real TMS7040 firmware speaks over `$FF7E`, per the Tandy Speech/Sound
//! Cartridge Owner's Manual (26-3144) Appendix A: command bytes that load
//! data into an 8×64-byte buffer RAM and execute it. Sound-data streams,
//! register-string streams, and direct AY register access (`$AF`) are fully
//! functional and drive the [`Ay8913`] PSG. Speech, allophone, and SP0256
//! command bytes are parsed just enough to keep the state machine in sync
//! (their LOAD variants still fill the buffer RAM per the flat-RAM model
//! below) but their EXECUTE variants are no-ops — no SP0256 is emulated, so
//! there is nothing to make them audible.
//!
//! See [`dispatch_command`](Ssc::dispatch_command) for the top-level command
//! dispatch and `docs/ssc-spec.md` for the full protocol writeup, including
//! every judgment call this implementation had to make where the manual
//! doesn't fully specify behavior.

use crate::ay8913::{Ay8913, mixer, reg as ay_reg};
use crate::cart::{Cartridge, IO_OPEN_BUS};

/// Register addresses (MAME `coco_ssc.cpp`; SEB Unravelled II Appendix A;
/// `docs/cartridges.md` "Carts can decode addresses outside SCS").
pub mod reg {
    /// SP0256 reset control (write) / always `0xFF` (read).
    pub const RESET: u16 = 0xFF7D;
    /// Host command latch (write) / status byte (read).
    pub const DATA: u16 = 0xFF7E;
}

/// `$FF7D` write: only bit 0 is decoded (real hardware wires it to the
/// SP0256's RESET pin).
const RESET_BIT: u8 = 0x01;

/// `$FF7E` status byte bit layout (MAME `coco_ssc_device::ff7e_r`).
mod status {
    /// Bits 4-0 read back set unconditionally — undocumented/unused status
    /// lines that MAME's real-hardware trace shows pulled high.
    pub const BASE: u8 = 0x1F;
    /// Bit 7: busy/ready. Set (1) when NOT busy; clear while a host byte is
    /// still "being processed".
    pub const NOT_BUSY: u8 = 0x80;
    /// Bit 6: SP0256 SBY ("standby" = idle/ready). No SP0256 is emulated, so
    /// this is always set — see [`super::Ssc`]'s doc comment. The
    /// execute-speech commands ([`cmd::EXEC_SPEECH_CONSECUTIVE_START`]-[`cmd::EXEC_SPEECH_CONSECUTIVE_END`],
    /// [`cmd::EXEC_SPEECH_INDIVIDUAL_START`]-[`cmd::EXEC_SPEECH_INDIVIDUAL_END`])
    /// are no-ops by design and never clear this bit, since there is no
    /// speech synthesis running to report busy.
    pub const SPEECH_READY: u8 = 0x40;
    /// Bit 5: Sound Activity Circuit output, *inverted* — 1 = quiet, 0 =
    /// sound is playing (MAME returns `!sound_active`).
    pub const QUIET: u8 = 0x20;
}

/// Synthetic hold time for `busy` after a `$FF7E` write, in E-clock cycles.
///
/// On real hardware the TMS7040 firmware clears busy (via a port-bit toggle)
/// once it has consumed the host byte — there's no fixed duration, it's
/// "whenever the firmware gets around to it". Without that firmware we hold
/// busy for a fixed, made-up window instead, long enough that software
/// polling the status byte observes a genuine busy period but short enough
/// not to stall a driver that spins on it. Not a hardware fact.
const BUSY_HOLD_CYCLES: u32 = 100;

/// AY-3-8913 master clock = 2× the CoCo E-clock (MAME `coco_ssc.cpp`: the
/// PSG and the TMS7040 CPU share one crystal, both clocked at twice the
/// bus's E-clock rate).
const AY_CLOCK_MULTIPLIER: u32 = 2;

// ---- Host byte protocol: command bytes -----------------------------------
//
// Every top-level command byte/range from the manual's Appendix A. All
// ranges are disjoint and exhaustive over `0x00, 0x80-0xFF`; `0x01-0x7F`
// (bit7 clear) is plain ASCII text-to-speech data in the default input mode
// and has no named constant here (handled by `dispatch_command`'s fallback
// arm). See `docs/ssc-spec.md`'s command table for the condensed version of
// this and citations.

/// Command byte/range constants (Tandy Speech/Sound Cartridge Owner's
/// Manual, 26-3144, Appendix A). Every "load N..=7" / "load individual N"
/// pair encodes the target buffer as `byte - START`.
pub mod cmd {
    /// Stop all sound (and, on real hardware, speech) immediately. Does NOT
    /// clear buffer RAM. Identical in this implementation to
    /// [`STOP_ALL_SOUND_ALT`].
    pub const STOP_ALL_SOUND: u8 = 0x00;

    /// Load speech string into buffers `N..=7` (consecutive), terminator
    /// [`super::terminator::SPEECH`]. `N = byte - LOAD_SPEECH_CONSECUTIVE_START`.
    pub const LOAD_SPEECH_CONSECUTIVE_START: u8 = 0x80;
    pub const LOAD_SPEECH_CONSECUTIVE_END: u8 = 0x87;
    /// Load sound data into buffers `N..=7` (consecutive), terminator
    /// [`super::terminator::SOUND`]. `N = byte - LOAD_SOUND_CONSECUTIVE_START`.
    pub const LOAD_SOUND_CONSECUTIVE_START: u8 = 0x88;
    pub const LOAD_SOUND_CONSECUTIVE_END: u8 = 0x8E;
    /// Load timer base value: exactly one postbyte (0-255), written to
    /// [`super::Ssc::timer_base`] directly, NOT to buffer RAM. See
    /// `docs/ssc-spec.md`'s timing section.
    pub const LOAD_TIMER_BASE: u8 = 0x8F;

    /// Load speech string into buffer `N` only (individual), terminator
    /// [`super::terminator::SPEECH`]. `N = byte - LOAD_SPEECH_INDIVIDUAL_START`.
    pub const LOAD_SPEECH_INDIVIDUAL_START: u8 = 0x90;
    pub const LOAD_SPEECH_INDIVIDUAL_END: u8 = 0x97;
    /// Load sound data into buffer `N` only (individual), terminator
    /// [`super::terminator::SOUND`]. `N = byte - LOAD_SOUND_INDIVIDUAL_START`.
    pub const LOAD_SOUND_INDIVIDUAL_START: u8 = 0x98;
    pub const LOAD_SOUND_INDIVIDUAL_END: u8 = 0x9F;

    /// Load allophone address stream into buffers `N..=7` (consecutive),
    /// terminator [`super::terminator::SOUND`].
    /// `N = byte - LOAD_ALLOPHONE_CONSECUTIVE_START`.
    pub const LOAD_ALLOPHONE_CONSECUTIVE_START: u8 = 0xA0;
    pub const LOAD_ALLOPHONE_CONSECUTIVE_END: u8 = 0xA7;
    /// Load register string into buffers `N..=7` (consecutive), terminator
    /// [`super::terminator::SOUND`]. `N = byte - LOAD_REGISTER_CONSECUTIVE_START`.
    pub const LOAD_REGISTER_CONSECUTIVE_START: u8 = 0xA8;
    pub const LOAD_REGISTER_CONSECUTIVE_END: u8 = 0xAE;
    /// Enter/exit direct access mode (register/value byte pairs poked
    /// straight into the AY). See [`super::DirectMode`].
    pub const DIRECT_ACCESS_TOGGLE: u8 = 0xAF;

    /// Load allophone address stream into buffer `N` only (individual),
    /// terminator [`super::terminator::SOUND`].
    /// `N = byte - LOAD_ALLOPHONE_INDIVIDUAL_START`.
    pub const LOAD_ALLOPHONE_INDIVIDUAL_START: u8 = 0xB0;
    pub const LOAD_ALLOPHONE_INDIVIDUAL_END: u8 = 0xB7;
    /// Load register string into buffer `N` only (individual), terminator
    /// [`super::terminator::SOUND`]. `N = byte - LOAD_REGISTER_INDIVIDUAL_START`.
    pub const LOAD_REGISTER_INDIVIDUAL_START: u8 = 0xB8;
    pub const LOAD_REGISTER_INDIVIDUAL_END: u8 = 0xBF;

    /// Execute speech string from buffers `N..=7` (consecutive) — no-op, no
    /// SP0256 emulated. `N = byte - EXEC_SPEECH_CONSECUTIVE_START`.
    pub const EXEC_SPEECH_CONSECUTIVE_START: u8 = 0xC0;
    pub const EXEC_SPEECH_CONSECUTIVE_END: u8 = 0xC6;
    /// Abort all speech — no-op, no SP0256 emulated.
    pub const ABORT_ALL_SPEECH: u8 = 0xC7;
    /// Execute sound data from buffers `N..=7` (consecutive): runs the sound
    /// engine. `N = byte - EXEC_SOUND_CONSECUTIVE_START`.
    pub const EXEC_SOUND_CONSECUTIVE_START: u8 = 0xC8;
    pub const EXEC_SOUND_CONSECUTIVE_END: u8 = 0xCE;
    /// Stop all sound. Identical in this implementation to
    /// [`STOP_ALL_SOUND`].
    pub const STOP_ALL_SOUND_ALT: u8 = 0xCF;

    /// Execute speech string from buffer `N` only (individual) — no-op, no
    /// SP0256 emulated. `N = byte - EXEC_SPEECH_INDIVIDUAL_START`.
    pub const EXEC_SPEECH_INDIVIDUAL_START: u8 = 0xD0;
    pub const EXEC_SPEECH_INDIVIDUAL_END: u8 = 0xD7;
    /// Execute sound data from buffer `N` only (individual): runs the sound
    /// engine. `N = byte - EXEC_SOUND_INDIVIDUAL_START`.
    pub const EXEC_SOUND_INDIVIDUAL_START: u8 = 0xD8;
    pub const EXEC_SOUND_INDIVIDUAL_END: u8 = 0xDF;

    /// Execute allophone address stream from buffers `N..=7` (consecutive)
    /// — no-op, no SP0256 emulated. `N = byte - EXEC_ALLOPHONE_CONSECUTIVE_START`.
    pub const EXEC_ALLOPHONE_CONSECUTIVE_START: u8 = 0xE0;
    pub const EXEC_ALLOPHONE_CONSECUTIVE_END: u8 = 0xE7;
    /// Execute register string from buffers `N..=7` (consecutive): writes
    /// `(register, value)` pairs straight to the AY.
    /// `N = byte - EXEC_REGISTER_CONSECUTIVE_START`.
    pub const EXEC_REGISTER_CONSECUTIVE_START: u8 = 0xE8;
    pub const EXEC_REGISTER_CONSECUTIVE_END: u8 = 0xEF;

    /// Execute allophone address stream from buffer `N` only (individual) —
    /// no-op, no SP0256 emulated. `N = byte - EXEC_ALLOPHONE_INDIVIDUAL_START`.
    pub const EXEC_ALLOPHONE_INDIVIDUAL_START: u8 = 0xF0;
    pub const EXEC_ALLOPHONE_INDIVIDUAL_END: u8 = 0xF7;
    /// Execute register string from buffer `N` only (individual): writes
    /// `(register, value)` pairs straight to the AY.
    /// `N = byte - EXEC_REGISTER_INDIVIDUAL_START`.
    pub const EXEC_REGISTER_INDIVIDUAL_START: u8 = 0xF8;
    pub const EXEC_REGISTER_INDIVIDUAL_END: u8 = 0xFF;
}

/// Terminator bytes that end a buffer-RAM LOAD (and, for sound data, a
/// re-scan at EXECUTE time — see the [`ram`] module doc comment).
pub mod terminator {
    /// Ends a speech-string load.
    pub const SPEECH: u8 = 0x0D;
    /// Ends a sound-data, allophone, or register-string load (and gates
    /// sound-data/register-string EXECUTE re-scanning — see [`super::ram`]).
    pub const SOUND: u8 = 0xFF;
}

/// Flat buffer RAM layout: 8 buffers of 64 bytes each, buffer `N` occupying
/// offset `N*BUFFER_SIZE..(N+1)*BUFFER_SIZE`. Buffer contents are untyped
/// raw bytes — whatever LOAD command filled a buffer, any later EXECUTE
/// command reads it back the same way, with no cross-checking of "was this
/// loaded as sound data".
///
/// **RAM prefills to [`super::RAM_RESET_BYTE`] (`0xFF`), not zero.** This is
/// a judgment call, not stated by the manual — but it is the only value
/// consistent with how EXECUTE re-scans a buffer: EXECUTE has no separate
/// "how many bytes did the LOAD actually store" bookkeeping, it just
/// re-scans from the buffer's start looking for its own terminator/stopping
/// condition. Since [`terminator::SOUND`] (`0xFF`) is also the
/// never-written-since-reset RAM value, scanning naturally stops exactly
/// where the load stopped. If RAM were zero-filled instead, an unwritten
/// byte one past a short sound-data load would misparse as a spurious
/// `0x00`-opcode (tone A) group and corrupt playback. See
/// `docs/ssc-spec.md` for the full rationale and a known unhandled edge
/// case (a shorter reload over a buffer region previously filled by a
/// longer one can leave stale non-terminator bytes just past the new
/// cursor — not worth engineering around).
pub mod ram {
    pub const BUFFER_COUNT: usize = 8;
    pub const BUFFER_SIZE: usize = 64;
    pub const SIZE: usize = BUFFER_COUNT * BUFFER_SIZE;
}

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

    /// Byte length of a group given its 3-bit opcode (already shifted/masked
    /// via [`OPCODE_SHIFT`]/[`OPCODE_MASK`]): 3 for noise groups, 4 for
    /// every other opcode (tone: `[amp, coarse, fine, duration]`; envelope:
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
/// plausible note/effect durations, exactly like [`BUSY_HOLD_CYCLES`].
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

/// Buffer RAM initial/reset fill value — see the [`ram`] module doc comment
/// for why this must be [`terminator::SOUND`] (`0xFF`) and not zero.
const RAM_RESET_BYTE: u8 = terminator::SOUND;

// ---- Host byte protocol: dispatch state ------------------------------------

/// Which "mode" the next accepted `$FF7E` byte is interpreted in.
#[derive(Clone, Copy)]
enum Mode {
    /// Ready for a fresh top-level command byte (or plain ASCII
    /// text-to-speech data, `0x01-0x7F`, consumed and discarded).
    Idle,
    /// Mid buffer-RAM load (a `0x8x`-`0xBx` LOAD command), accumulating
    /// bytes until the terminator or capacity — see [`Load`].
    Loading(Load),
    /// `$AF` direct-access mode: alternating register-number/value byte
    /// pairs poked straight into the AY — see [`DirectMode`].
    Direct(DirectMode),
    /// `$8F` was just received; the next accepted byte is the timer-base
    /// postbyte, not a fresh command.
    AwaitTimerBase,
}

/// In-progress buffer-RAM load state (see [`Mode::Loading`] and the [`ram`]
/// module doc comment's loading-state-machine rules).
#[derive(Clone, Copy)]
struct Load {
    /// [`terminator::SPEECH`] or [`terminator::SOUND`], depending on which
    /// command started this load.
    terminator: u8,
    /// Next RAM offset to write.
    cursor: usize,
    /// One past the highest offset this load may write (consecutive loads:
    /// [`ram::SIZE`]; individual loads: the end of the target buffer).
    cap: usize,
}

/// `$AF` direct-access sub-state: alternates between expecting a register
/// number and expecting that register's value.
#[derive(Clone, Copy)]
enum DirectMode {
    /// Next byte is a register number, unless it's [`terminator::SOUND`]
    /// (`0xFF`), which exits direct-access mode instead — the manual's "FF
    /// hex" terminator only applies at this pair-start position.
    Register,
    /// Next byte is the value to write to register `.0` (any byte value,
    /// including `0xFF` — it is NOT a terminator here).
    Value(u8),
}

/// Sequential sound-data playback engine: a single active cursor through one
/// linear byte stream (buffer RAM), one group "playing" (gating the next
/// group's processing) for its scheduled duration at a time. A new
/// execute-sound-data command replaces whatever stream was previously
/// running — there is no queueing or concurrent-channel scheduling here
/// (real hardware achieves simultaneous multi-channel playback via the
/// separate, un-timed register-string LOAD/EXECUTE mechanism instead).
#[derive(Clone, Copy, Default)]
struct Engine {
    /// Whether the engine is currently advancing through a stream. Cleared
    /// at end-of-stream (terminator, incomplete trailing group, or capacity
    /// exhaustion) or by an explicit stop command — the engine simply stops
    /// advancing; it does NOT silence the AY on its own (see
    /// [`Ssc::advance_engine`]).
    active: bool,
    /// Next RAM offset to parse a group's opcode byte from.
    cursor: usize,
    /// One past the highest offset this stream may read from (consecutive:
    /// [`ram::SIZE`]; individual: the end of the target buffer).
    cap: usize,
    /// Tracks the most recent group's raw 4-bit amplitude field, for noise
    /// groups' "R" reuse flag. Reset to 0 at the start of every
    /// execute-sound-data command.
    last_amplitude_nibble: u8,
    /// E-clock cycles remaining before the current group's duration elapses
    /// and [`Ssc::advance_engine`] runs again.
    duration_countdown: u32,
}

// ---- Sound Activity Circuit --------------------------------------------------
//
// An envelope follower on the cartridge's own (pre-mux) audio output, purely
// so `$FF7E` bit 5 can report whether the PSG is making sound. Runs on every
// `audio_sample` call regardless of whether the CoCo's sound mux is actually
// listening to the cartridge (MAME `coco_ssc.cpp` `sac_update`; time
// constants below are tuned for MAME's own ~44.1 kHz-ish audio-stream
// sampling rate — the audio grid's ~62.9 kHz call rate is close enough
// that the same constants serve, and much closer than the old
// once-per-scanline 15.7 kHz rate was).
mod sac {
    /// One-pole DC-blocking high-pass filter coefficient:
    /// `y = ALPHA * (y_prev + x - x_prev)`.
    pub const HPF_ALPHA: f32 = 0.99;
    /// Leaky-integrator attack rate (envelope rising toward a louder
    /// rectified sample).
    pub const ATTACK_COEFF: f32 = 0.0026;
    /// Leaky-integrator decay rate (envelope falling toward a quieter one).
    pub const DECAY_COEFF: f32 = 0.0003;
    /// Envelope level above which sound is considered "active".
    pub const THRESH_ON: f32 = 0.05;
    /// Envelope level below which sound is considered "quiet" again
    /// (hysteresis: lower than [`THRESH_ON`] so the status bit doesn't
    /// chatter around a single threshold).
    pub const THRESH_OFF: f32 = 0.01;
}

/// The Sound/Speech Cartridge: `$FF7D`/`$FF7E` handshake, host-byte protocol
/// interpreter (see the module doc comment), and an AY-3-8913 mixed into the
/// machine's audio output.
///
/// No SP0256 speech synthesizer is modelled, so [`status::SPEECH_READY`] is
/// always reported set (idle/ready) — see the module doc comment.
pub struct Ssc {
    ay: Ay8913,
    /// Bit 0 of the last byte written to `$FF7D`, for falling-edge detection
    /// on the next write. Power-on-reset starts clear so the very first
    /// `$FF7D` write (even if it's bit0=0) is never itself treated as a
    /// falling edge — only a later 1-then-0 pair is (MAME
    /// `coco_ssc_device::device_reset` primes `m_reset_line` the same way).
    prev_reset_bit0: bool,
    /// Last byte latched from a `$FF7E` write (the "Port A latch" the real
    /// TMS7040 firmware reads and interprets as a command/data byte).
    /// Stored for tests/debug; the actual interpretation happens in
    /// [`Ssc::dispatch`], invoked synchronously from [`Ssc::write_data`].
    host_latch: u8,
    busy: bool,
    /// E-clock cycles remaining before [`Ssc::busy`] synthetically clears —
    /// see [`BUSY_HOLD_CYCLES`].
    busy_countdown: u32,
    // Sound Activity Circuit state (see the `sac` module doc comment).
    sac_hpf_prev_in: f32,
    sac_hpf_prev_out: f32,
    sac_envelope: f32,
    /// True while the SAC considers the cartridge's own output "playing"
    /// (drives `$FF7E` bit 5, inverted).
    sac_sound_active: bool,

    // ---- Host byte protocol state (see the module doc comment) ----------
    /// Flat 8×64-byte buffer RAM — see the [`ram`] module doc comment.
    ram: [u8; ram::SIZE],
    /// Top-level protocol dispatch state — see [`Mode`].
    mode: Mode,
    /// `$8F`'s postbyte: scales every subsequent sound-data event's
    /// duration — see [`timing`].
    timer_base: u8,
    /// The sequential sound-data playback engine — see [`Engine`].
    engine: Engine,
}

impl Default for Ssc {
    fn default() -> Self {
        Self::new()
    }
}

impl Ssc {
    pub fn new() -> Self {
        Self {
            ay: Ay8913::new(),
            prev_reset_bit0: false,
            host_latch: 0,
            busy: false,
            busy_countdown: 0,
            sac_hpf_prev_in: 0.0,
            sac_hpf_prev_out: 0.0,
            sac_envelope: 0.0,
            sac_sound_active: false,
            ram: [RAM_RESET_BYTE; ram::SIZE],
            mode: Mode::Idle,
            timer_base: timing::DEFAULT_TIMER_BASE,
            engine: Engine::default(),
        }
    }

    /// Direct AY-3-8913 register write — bypasses the host-byte protocol
    /// (used internally by the protocol interpreter itself, and exposed for
    /// tests/debugging).
    pub fn ay_write(&mut self, reg: u8, val: u8) {
        self.ay.write_reg(reg, val);
    }

    /// Direct AY-3-8913 register read (see [`Ssc::ay_write`]).
    pub fn ay_read(&mut self, reg: u8) -> u8 {
        self.ay.read_reg(reg)
    }

    /// The last byte latched from a `$FF7E` write, for tests/debug.
    pub fn host_latch(&self) -> u8 {
        self.host_latch
    }

    fn write_reset(&mut self, val: u8) {
        let bit0 = val & RESET_BIT != 0;
        let falling_edge = self.prev_reset_bit0 && !bit0;
        self.prev_reset_bit0 = bit0;
        if falling_edge {
            // Real hardware's falling edge on the SP0256 RESET pin also
            // resets the AY (MAME `coco_ssc_device`) and leaves the firmware
            // ready for a new command.
            self.ay.reset();
            self.busy = false;
            self.busy_countdown = 0;
            self.reset_protocol_state();
        }
        // bit0=1 alone: real hardware asserts the SP0256's RESET pin, which
        // we don't emulate (no SP0256) -- no-op.
    }

    /// Resets buffer RAM, dispatch mode, timer base, and the sound engine —
    /// shared by [`Ssc::write_reset`]'s falling-edge handler and the
    /// [`Cartridge::reset`] trait method. Does NOT touch the AY, busy
    /// handshake, or SAC state — callers already handle those themselves.
    fn reset_protocol_state(&mut self) {
        self.ram = [RAM_RESET_BYTE; ram::SIZE];
        self.mode = Mode::Idle;
        self.timer_base = timing::DEFAULT_TIMER_BASE;
        self.engine = Engine::default();
    }

    /// `$FF7E` write: the host-byte protocol entry point.
    ///
    /// Per the manual (page 10): "If you try to transfer data to the S/SC
    /// while bit 7 is low, you lose all the data you send until the bit
    /// resets." So while [`Ssc::busy`] is already set, an incoming byte is
    /// discarded entirely — not latched, not fed to the protocol state
    /// machine, and it does not restart the busy hold window. Every byte
    /// that IS accepted (command bytes, load-data bytes, direct-access
    /// register/value bytes, the `$8F` postbyte alike) is processed through
    /// [`Ssc::dispatch`] synchronously, right here.
    fn write_data(&mut self, val: u8) {
        if self.busy {
            return;
        }
        self.host_latch = val;
        self.busy = true;
        self.busy_countdown = BUSY_HOLD_CYCLES;
        // Real hardware also asserts the TMS7000's INT3 here, waking the
        // firmware to consume the byte — not modelled (no TMS7000 core); we
        // interpret the byte synchronously instead.
        self.dispatch(val);
    }

    fn read_data(&self) -> u8 {
        let mut s = status::BASE | status::SPEECH_READY;
        if !self.busy {
            s |= status::NOT_BUSY;
        }
        if !self.sac_sound_active {
            s |= status::QUIET;
        }
        s
    }

    // ---- Host byte protocol: dispatch --------------------------------------

    /// Routes an accepted `$FF7E` byte to the current [`Mode`]'s handler.
    fn dispatch(&mut self, byte: u8) {
        match self.mode {
            Mode::Loading(load) => self.feed_load(load, byte),
            Mode::Direct(direct) => self.feed_direct(direct, byte),
            Mode::AwaitTimerBase => {
                self.timer_base = byte;
                self.mode = Mode::Idle;
            }
            Mode::Idle => self.dispatch_command(byte),
        }
    }

    /// Buffer-RAM load loop (see the [`ram`] module doc comment's
    /// loading-state-machine rules).
    fn feed_load(&mut self, load: Load, byte: u8) {
        if byte == load.terminator {
            self.mode = Mode::Idle;
            return;
        }
        if load.cursor >= load.cap {
            // Capacity exhausted without ever seeing the terminator: the
            // load ends WITHOUT storing this overflowing byte, and — since
            // the manual says the protocol "reverts to normal input mode" —
            // this same byte is immediately re-dispatched as if freshly
            // received in Idle mode.
            self.mode = Mode::Idle;
            self.dispatch(byte);
            return;
        }
        self.ram[load.cursor] = byte;
        self.mode = Mode::Loading(Load { cursor: load.cursor + 1, ..load });
    }

    /// `$AF` direct-access register/value alternation (see [`DirectMode`]).
    fn feed_direct(&mut self, direct: DirectMode, byte: u8) {
        match direct {
            DirectMode::Register => {
                if byte == terminator::SOUND {
                    self.mode = Mode::Idle;
                } else {
                    self.mode = Mode::Direct(DirectMode::Value(byte));
                }
            }
            DirectMode::Value(register) => {
                self.ay_write(register, byte);
                self.mode = Mode::Direct(DirectMode::Register);
            }
        }
    }

    /// Top-level command dispatch (Idle mode only) — every command
    /// byte/range from [`cmd`]. See the module doc comment and
    /// `docs/ssc-spec.md` for the full protocol writeup.
    fn dispatch_command(&mut self, byte: u8) {
        match byte {
            cmd::STOP_ALL_SOUND | cmd::STOP_ALL_SOUND_ALT => self.stop_all_sound(),

            cmd::LOAD_SPEECH_CONSECUTIVE_START..=cmd::LOAD_SPEECH_CONSECUTIVE_END => {
                let n = byte - cmd::LOAD_SPEECH_CONSECUTIVE_START;
                self.start_load_consecutive(terminator::SPEECH, n);
            }
            cmd::LOAD_SOUND_CONSECUTIVE_START..=cmd::LOAD_SOUND_CONSECUTIVE_END => {
                let n = byte - cmd::LOAD_SOUND_CONSECUTIVE_START;
                self.start_load_consecutive(terminator::SOUND, n);
            }
            cmd::LOAD_TIMER_BASE => {
                self.mode = Mode::AwaitTimerBase;
            }

            cmd::LOAD_SPEECH_INDIVIDUAL_START..=cmd::LOAD_SPEECH_INDIVIDUAL_END => {
                let n = byte - cmd::LOAD_SPEECH_INDIVIDUAL_START;
                self.start_load_individual(terminator::SPEECH, n);
            }
            cmd::LOAD_SOUND_INDIVIDUAL_START..=cmd::LOAD_SOUND_INDIVIDUAL_END => {
                let n = byte - cmd::LOAD_SOUND_INDIVIDUAL_START;
                self.start_load_individual(terminator::SOUND, n);
            }

            cmd::LOAD_ALLOPHONE_CONSECUTIVE_START..=cmd::LOAD_ALLOPHONE_CONSECUTIVE_END => {
                let n = byte - cmd::LOAD_ALLOPHONE_CONSECUTIVE_START;
                self.start_load_consecutive(terminator::SOUND, n);
            }
            cmd::LOAD_REGISTER_CONSECUTIVE_START..=cmd::LOAD_REGISTER_CONSECUTIVE_END => {
                let n = byte - cmd::LOAD_REGISTER_CONSECUTIVE_START;
                self.start_load_consecutive(terminator::SOUND, n);
            }
            cmd::DIRECT_ACCESS_TOGGLE => {
                self.mode = Mode::Direct(DirectMode::Register);
            }

            cmd::LOAD_ALLOPHONE_INDIVIDUAL_START..=cmd::LOAD_ALLOPHONE_INDIVIDUAL_END => {
                let n = byte - cmd::LOAD_ALLOPHONE_INDIVIDUAL_START;
                self.start_load_individual(terminator::SOUND, n);
            }
            cmd::LOAD_REGISTER_INDIVIDUAL_START..=cmd::LOAD_REGISTER_INDIVIDUAL_END => {
                let n = byte - cmd::LOAD_REGISTER_INDIVIDUAL_START;
                self.start_load_individual(terminator::SOUND, n);
            }

            cmd::EXEC_SOUND_CONSECUTIVE_START..=cmd::EXEC_SOUND_CONSECUTIVE_END => {
                let n = byte - cmd::EXEC_SOUND_CONSECUTIVE_START;
                let start = n as usize * ram::BUFFER_SIZE;
                self.start_sound_execute(start, ram::SIZE);
            }
            cmd::EXEC_SOUND_INDIVIDUAL_START..=cmd::EXEC_SOUND_INDIVIDUAL_END => {
                let n = byte - cmd::EXEC_SOUND_INDIVIDUAL_START;
                let start = n as usize * ram::BUFFER_SIZE;
                self.start_sound_execute(start, start + ram::BUFFER_SIZE);
            }

            cmd::EXEC_REGISTER_CONSECUTIVE_START..=cmd::EXEC_REGISTER_CONSECUTIVE_END => {
                let n = byte - cmd::EXEC_REGISTER_CONSECUTIVE_START;
                let start = n as usize * ram::BUFFER_SIZE;
                self.execute_register_string(start, ram::SIZE);
            }
            cmd::EXEC_REGISTER_INDIVIDUAL_START..=cmd::EXEC_REGISTER_INDIVIDUAL_END => {
                let n = byte - cmd::EXEC_REGISTER_INDIVIDUAL_START;
                let start = n as usize * ram::BUFFER_SIZE;
                self.execute_register_string(start, start + ram::BUFFER_SIZE);
            }

            cmd::EXEC_SPEECH_CONSECUTIVE_START..=cmd::EXEC_SPEECH_CONSECUTIVE_END
            | cmd::ABORT_ALL_SPEECH
            | cmd::EXEC_SPEECH_INDIVIDUAL_START..=cmd::EXEC_SPEECH_INDIVIDUAL_END
            | cmd::EXEC_ALLOPHONE_CONSECUTIVE_START..=cmd::EXEC_ALLOPHONE_CONSECUTIVE_END
            | cmd::EXEC_ALLOPHONE_INDIVIDUAL_START..=cmd::EXEC_ALLOPHONE_INDIVIDUAL_END => {
                // Speech/allophone execute commands: no-op, no SP0256
                // emulated (see the module doc comment).
            }

            // `0x01-0x7F` (bit7 clear): plain ASCII text-to-speech data in
            // the default input mode. Consumed and discarded — this also
            // covers `0x0D` arriving here, which has no special effect
            // beyond being discarded, since nothing accumulates or speaks
            // it in this implementation.
            _ => {}
        }
    }

    /// Starts a consecutive buffer-RAM load at buffer `n`'s offset, capacity
    /// through the end of RAM (may spill into later buffers).
    fn start_load_consecutive(&mut self, terminator: u8, n: u8) {
        let cursor = n as usize * ram::BUFFER_SIZE;
        self.mode = Mode::Loading(Load { terminator, cursor, cap: ram::SIZE });
    }

    /// Starts an individual buffer-RAM load confined to buffer `n` only.
    fn start_load_individual(&mut self, terminator: u8, n: u8) {
        let cursor = n as usize * ram::BUFFER_SIZE;
        self.mode = Mode::Loading(Load { terminator, cursor, cap: cursor + ram::BUFFER_SIZE });
    }

    /// `$00`/`$CF` "stop all sound": halts the engine (it stops advancing)
    /// and writes 0 to all three AY channel volumes — a true off, unlike
    /// natural end-of-stream (see [`Ssc::advance_engine`]'s doc comment).
    /// `$00` additionally stops speech on real hardware, a no-op here (no
    /// SP0256). Both commands are otherwise identical in this
    /// implementation.
    fn stop_all_sound(&mut self) {
        self.engine = Engine::default();
        self.ay_write(ay_reg::VOL_A, 0);
        self.ay_write(ay_reg::VOL_B, 0);
        self.ay_write(ay_reg::VOL_C, 0);
    }

    /// Starts a sound-data EXECUTE stream: resets the engine to the given
    /// window and synchronously parses/programs the first group (subsequent
    /// groups advance from [`Cartridge::tick`] via [`Ssc::tick_engine`]).
    fn start_sound_execute(&mut self, start: usize, cap: usize) {
        self.engine = Engine { active: true, cursor: start, cap, last_amplitude_nibble: 0, duration_countdown: 0 };
        self.advance_engine();
    }

    /// Executes a register-string stream: `(register, value)` pairs applied
    /// straight to the AY, immediately, with no timing — unlike sound-data
    /// groups this is not scheduled through [`Engine`], since the manual
    /// describes register strings as unbuffered "on the fly" pokes (same
    /// mechanism as `$AF` direct-access, just sourced from RAM instead of
    /// live host bytes). `$FF` at a pair-start position ends the stream,
    /// mirroring the sound-data terminator rule; a dangling odd byte with no
    /// paired value at the end of the window is dropped rather than
    /// misapplied (the manual doesn't address this case for register
    /// strings specifically — treated the same as sound-data's "incomplete
    /// trailing group never executes" rule, as the smallest consistent
    /// choice).
    fn execute_register_string(&mut self, start: usize, cap: usize) {
        let mut cursor = start;
        while cursor < cap {
            let register = self.ram[cursor];
            if register == terminator::SOUND {
                break;
            }
            if cursor + 1 >= cap {
                break;
            }
            let value = self.ram[cursor + 1];
            self.ay_write(register, value);
            cursor += 2;
        }
    }

    // ---- Sound-data playback engine ----------------------------------------

    /// Parses and programs exactly one group at [`Engine::cursor`], then
    /// schedules its duration. Called synchronously by
    /// [`Ssc::start_sound_execute`] and again by [`Ssc::tick_engine`] each
    /// time a group's duration elapses.
    ///
    /// End-of-stream (terminator found, or the next group doesn't fully fit
    /// before [`Engine::cap`]) simply clears [`Engine::active`] — the engine
    /// stops advancing. It does NOT silence the AY: whatever registers the
    /// last successfully-processed group programmed remain exactly as set,
    /// indefinitely, until something else overwrites them (a new execute
    /// command, or an explicit stop via [`Ssc::stop_all_sound`]).
    fn advance_engine(&mut self) {
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
            group::TONE_A | group::TONE_B | group::TONE_C => self.process_tone_group(cursor, opcode),
            group::NOISE_A | group::NOISE_B | group::NOISE_C => self.process_noise_group(cursor, opcode),
            group::ENVELOPE_LOW | group::ENVELOPE_HIGH => self.process_standalone_envelope_group(cursor),
            _ => unreachable!("opcode is masked to 3 bits (0-7); all 8 values are handled above"),
        }
    }

    /// Programs a tone group (4 bytes: `[op|M|amp, coarse, fine, duration]`)
    /// at `at`, chains an immediately-following M=1 envelope group if
    /// present, and schedules the resulting event's duration.
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
    /// `at`, applying the "R" reuse-previous-amplitude flag, chains an
    /// immediately-following M=1 envelope group if present, and schedules
    /// the resulting event's duration.
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
        let amp = if reuse { self.engine.last_amplitude_nibble } else { raw_amp };

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

    /// Defensive fallback for a standalone envelope group encountered at the
    /// top of [`Ssc::advance_engine`]'s dispatch — i.e. one NOT immediately
    /// preceded by an M=1 tone/noise group, which is not a documented manual
    /// behavior (envelope groups only ever appear chained after an M=1
    /// group). Programmed as its own one-group event anyway, purely so a
    /// malformed stream can't desync or infinite-loop the engine.
    fn process_standalone_envelope_group(&mut self, at: usize) {
        let cap = self.engine.cap;
        let (duration, len) = self
            .peek_chained_envelope(at, cap)
            .expect("advance_engine already verified an envelope opcode with a full group available");
        self.engine.cursor = at + len;
        self.engine.duration_countdown = timing::duration_cycles(duration, self.timer_base);
    }

    /// If a well-formed envelope group (4 bytes: `[shape, coarse, fine,
    /// duration]`) starts at `after` and fits within `cap`, programs its AY
    /// registers immediately and returns `(duration_byte, 4)`. Returns
    /// `None` (without touching the AY) if there isn't room or the byte at
    /// `after` isn't an envelope opcode. Shared by the tone/noise M=1
    /// chaining path and [`Ssc::process_standalone_envelope_group`].
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

    /// Read-modify-write R7 so that channel `ch`'s tone/noise generators are
    /// enabled/disabled as requested, without disturbing the other two
    /// channels' mixer bits.
    ///
    /// Judgment call, not literally stated by the manual: the manual's own
    /// "TUNE1"/"TUNE2" demo DATA streams (tone-A-only) never poke R7
    /// directly, so without this auto-enable those examples would play
    /// silently. Required for any sound-data stream that doesn't itself
    /// include a register-string command to set up the mixer first.
    fn set_mixer_channel(&mut self, ch: usize, tone_enabled: bool, noise_enabled: bool) {
        let mut mixer_val = self.ay_read(ay_reg::MIXER);
        let tone_bit = 1 << (mixer::TONE_DISABLE_SHIFT + ch as u8);
        let noise_bit = 1 << (mixer::NOISE_DISABLE_SHIFT + ch as u8);
        // Mixer bits are active-low enable: clear the bit to enable that
        // generator for the channel, set it to disable.
        mixer_val = if tone_enabled { mixer_val & !tone_bit } else { mixer_val | tone_bit };
        mixer_val = if noise_enabled { mixer_val & !noise_bit } else { mixer_val | noise_bit };
        self.ay_write(ay_reg::MIXER, mixer_val);
    }

    /// Advances the sound-data engine by `cycles` E-clock cycles, called
    /// from [`Cartridge::tick`]. If the current group's duration has
    /// elapsed (`cycles >= duration_countdown`), [`Ssc::advance_engine`]
    /// runs immediately — no remainder is carried into the next event's
    /// countdown, matching the same "keep it simple" tradeoff as
    /// [`BUSY_HOLD_CYCLES`]'s handling elsewhere in this file.
    fn tick_engine(&mut self, cycles: u32) {
        if !self.engine.active {
            return;
        }
        if cycles >= self.engine.duration_countdown {
            self.advance_engine();
        } else {
            self.engine.duration_countdown -= cycles;
        }
    }

    /// Sound Activity Circuit envelope follower — see the `sac` module doc
    /// comment. Runs unconditionally on every [`Ssc::audio_sample`] call.
    fn update_sac(&mut self, x: f32) {
        let y = sac::HPF_ALPHA * (self.sac_hpf_prev_out + x - self.sac_hpf_prev_in);
        self.sac_hpf_prev_in = x;
        self.sac_hpf_prev_out = y;

        let rectified = y.abs();
        let coeff = if rectified > self.sac_envelope {
            sac::ATTACK_COEFF
        } else {
            sac::DECAY_COEFF
        };
        self.sac_envelope += coeff * (rectified - self.sac_envelope);

        if self.sac_envelope > sac::THRESH_ON {
            self.sac_sound_active = true;
        } else if self.sac_envelope < sac::THRESH_OFF {
            self.sac_sound_active = false;
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

impl Cartridge for Ssc {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            reg::RESET => 0xFF, // always, regardless of state (MAME `ff7d_r`)
            reg::DATA => self.read_data(),
            _ => IO_OPEN_BUS,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        match addr {
            reg::RESET => self.write_reset(val),
            reg::DATA => self.write_data(val),
            _ => {}
        }
    }

    fn tick(&mut self, cycles: u32) {
        if self.busy {
            self.busy_countdown = self.busy_countdown.saturating_sub(cycles);
            if self.busy_countdown == 0 {
                self.busy = false;
            }
        }
        self.tick_engine(cycles);
        self.ay.step(cycles * AY_CLOCK_MULTIPLIER);
    }

    fn as_ssc(&mut self) -> Option<&mut Ssc> {
        Some(self)
    }

    fn reset(&mut self) {
        self.ay.reset();
        self.prev_reset_bit0 = false;
        self.host_latch = 0;
        self.busy = false;
        self.busy_countdown = 0;
        self.sac_hpf_prev_in = 0.0;
        self.sac_hpf_prev_out = 0.0;
        self.sac_envelope = 0.0;
        self.sac_sound_active = false;
        self.reset_protocol_state();
    }

    /// Drains the AY's accumulated output and feeds the Sound Activity
    /// Circuit — unconditionally, since `$FF7E` bit 5 must reflect the
    /// cartridge's own output regardless of whether `SystemBus`'s sound mux
    /// is currently selecting it (see the `sac` module doc comment).
    /// [`SystemBus::sound_sample`](crate::bus::SystemBus::sound_sample)
    /// calls this exactly once per sample and only mixes the returned value
    /// in when the mux selects the cartridge input.
    fn audio_sample(&mut self) -> f32 {
        let out = self.ay.drain();
        self.update_sac(out);
        out
    }
}
