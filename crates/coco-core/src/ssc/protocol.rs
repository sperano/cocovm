//! Host-byte protocol interpreter for `$FF7E` writes: top-level command
//! dispatch ([`SoundSpeechCartridge::dispatch_command`]), buffer-RAM loads
//! ([`SoundSpeechCartridge::feed_load`]), and `$AF` direct-AY-access mode
//! ([`SoundSpeechCartridge::feed_direct`]). See the `ssc` module doc comment
//! and `docs/ssc-spec.md` for the full protocol writeup.

use serde::{Deserialize, Serialize};

use super::SoundSpeechCartridge;

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
    /// [`super::SoundSpeechCartridge::timer_base`] directly, NOT to buffer RAM. See
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
    /// straight into the AY). See [`super::protocol::DirectMode`].
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

/// Buffer RAM initial/reset fill value — see the [`ram`] module doc comment
/// for why this must be [`terminator::SOUND`] (`0xFF`) and not zero.
pub(super) const RAM_RESET_BYTE: u8 = terminator::SOUND;

// ---- Host byte protocol: dispatch state ------------------------------------

/// Which "mode" the next accepted `$FF7E` byte is interpreted in.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum Mode {
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
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) struct Load {
    /// [`terminator::SPEECH`] or [`terminator::SOUND`], depending on which
    /// command started this load.
    terminator: u8,
    /// Next RAM offset to write.
    pub(super) cursor: usize,
    /// One past the highest offset this load may write (consecutive loads:
    /// [`ram::SIZE`]; individual loads: the end of the target buffer).
    pub(super) cap: usize,
}

/// `$AF` direct-access sub-state: alternates between expecting a register
/// number and expecting that register's value.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(super) enum DirectMode {
    /// Next byte is a register number, unless it's [`terminator::SOUND`]
    /// (`0xFF`), which exits direct-access mode instead — the manual's "FF
    /// hex" terminator only applies at this pair-start position.
    Register,
    /// Next byte is the value to write to register `.0` (any byte value,
    /// including `0xFF` — it is NOT a terminator here).
    Value(u8),
}

impl SoundSpeechCartridge {
    // ---- Host byte protocol: dispatch --------------------------------------

    /// Routes an accepted `$FF7E` byte to the current [`Mode`]'s handler.
    pub(super) fn dispatch(&mut self, byte: u8) {
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
            // Capacity exhausted without a terminator: byte is dropped and
            // re-dispatched in Idle mode (manual: "reverts to normal input
            // mode").
            self.mode = Mode::Idle;
            self.dispatch(byte);
            return;
        }
        self.ram[load.cursor] = byte;
        self.mode = Mode::Loading(Load {
            cursor: load.cursor + 1,
            ..load
        });
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
    /// byte/range from [`cmd`]. See `docs/ssc-spec.md` for the full protocol writeup.
    fn dispatch_command(&mut self, byte: u8) {
        match byte {
            cmd::STOP_ALL_SOUND | cmd::STOP_ALL_SOUND_ALT => self.stop_all_sound(),

            // Covers every LOAD command plus $AF (DIRECT_ACCESS_TOGGLE), which falls in this range.
            cmd::LOAD_SPEECH_CONSECUTIVE_START..=cmd::LOAD_REGISTER_INDIVIDUAL_END => {
                self.dispatch_load_command(byte)
            }

            cmd::EXEC_SPEECH_CONSECUTIVE_START..=cmd::EXEC_REGISTER_INDIVIDUAL_END => {
                self.dispatch_exec_command(byte)
            }

            // 0x01-0x7F (bit7 clear): plain ASCII text-to-speech data, consumed and discarded.
            _ => {}
        }
    }

    /// The `$8x`/`$9x`/`$Ax`/`$Bx` half of [`SoundSpeechCartridge::dispatch_command`]:
    /// every LOAD command and `$8F`/`$AF`.
    fn dispatch_load_command(&mut self, byte: u8) {
        match byte {
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

            _ => unreachable!("caller only dispatches the LOAD range and $AF here"),
        }
    }

    /// The `$Cx`-`$Fx` half of [`SoundSpeechCartridge::dispatch_command`]: every
    /// EXECUTE command plus `$C7` abort-all-speech. Only the sound-data and
    /// register-string variants do anything.
    fn dispatch_exec_command(&mut self, byte: u8) {
        match byte {
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
                // Speech/allophone execute commands: no-op, no SP0256 emulated.
            }

            _ => unreachable!("caller only dispatches the EXECUTE range here"),
        }
    }

    /// Starts a consecutive buffer-RAM load at buffer `n`'s offset, capacity
    /// through the end of RAM (may spill into later buffers).
    fn start_load_consecutive(&mut self, terminator: u8, n: u8) {
        let cursor = n as usize * ram::BUFFER_SIZE;
        self.mode = Mode::Loading(Load {
            terminator,
            cursor,
            cap: ram::SIZE,
        });
    }

    /// Starts an individual buffer-RAM load confined to buffer `n` only.
    fn start_load_individual(&mut self, terminator: u8, n: u8) {
        let cursor = n as usize * ram::BUFFER_SIZE;
        self.mode = Mode::Loading(Load {
            terminator,
            cursor,
            cap: cursor + ram::BUFFER_SIZE,
        });
    }

    /// Executes a register-string stream: `(register, value)` pairs applied
    /// straight to the AY, immediately, with no timing (unbuffered, per the
    /// manual). `$FF` at a pair-start ends the stream; a dangling odd byte
    /// at the end is dropped.
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
}
