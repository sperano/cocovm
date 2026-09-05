//! The host-byte protocol the firmware speaks over `$FF7E`, as constants:
//! command bytes, terminators, buffer RAM geometry, and the sound-data event
//! layout (Tandy Speech/Sound Cartridge Owner's Manual, 26-3144, Appendix
//! A). The firmware interprets these; this crate only names them so tests
//! and tools can build streams. See `docs/ssc-spec.md`.

/// Command bytes. Every "load N..=7" / "load individual N" pair encodes the
/// target buffer as `byte - START`.
pub mod cmd {
    /// Stop all sound and speech immediately. Does NOT clear buffer RAM.
    pub const STOP_ALL_SOUND: u8 = 0x00;

    /// Load speech string (ASCII text) into buffers `N..=7`, terminator
    /// [`super::terminator::SPEECH`].
    pub const LOAD_SPEECH_CONSECUTIVE_START: u8 = 0x80;
    pub const LOAD_SPEECH_CONSECUTIVE_END: u8 = 0x87;
    /// Load sound data into buffers `N..=7`, terminator [`super::terminator::SOUND`].
    pub const LOAD_SOUND_CONSECUTIVE_START: u8 = 0x88;
    pub const LOAD_SOUND_CONSECUTIVE_END: u8 = 0x8E;
    /// Load timer base value: one postbyte (0-255) scaling every sound-data
    /// event's duration.
    pub const LOAD_TIMER_BASE: u8 = 0x8F;

    /// Load speech string into buffer `N` only.
    pub const LOAD_SPEECH_INDIVIDUAL_START: u8 = 0x90;
    pub const LOAD_SPEECH_INDIVIDUAL_END: u8 = 0x97;
    /// Load sound data into buffer `N` only.
    pub const LOAD_SOUND_INDIVIDUAL_START: u8 = 0x98;
    pub const LOAD_SOUND_INDIVIDUAL_END: u8 = 0x9F;

    /// Load allophone address stream into buffers `N..=7`.
    pub const LOAD_ALLOPHONE_CONSECUTIVE_START: u8 = 0xA0;
    pub const LOAD_ALLOPHONE_CONSECUTIVE_END: u8 = 0xA7;
    /// Load register string into buffers `N..=7`.
    pub const LOAD_REGISTER_CONSECUTIVE_START: u8 = 0xA8;
    pub const LOAD_REGISTER_CONSECUTIVE_END: u8 = 0xAE;
    /// Enter direct access mode: `(register, value)` pairs poked straight
    /// into the AY until an `$FF` where a register number is expected.
    pub const DIRECT_ACCESS_TOGGLE: u8 = 0xAF;

    /// Load allophone address stream into buffer `N` only.
    pub const LOAD_ALLOPHONE_INDIVIDUAL_START: u8 = 0xB0;
    pub const LOAD_ALLOPHONE_INDIVIDUAL_END: u8 = 0xB7;
    /// Load register string into buffer `N` only.
    pub const LOAD_REGISTER_INDIVIDUAL_START: u8 = 0xB8;
    pub const LOAD_REGISTER_INDIVIDUAL_END: u8 = 0xBF;

    /// Execute speech string (text-to-speech) from buffers `N..=7`.
    pub const EXEC_SPEECH_CONSECUTIVE_START: u8 = 0xC0;
    pub const EXEC_SPEECH_CONSECUTIVE_END: u8 = 0xC6;
    /// Abort all speech.
    pub const ABORT_ALL_SPEECH: u8 = 0xC7;
    /// Execute sound data from buffers `N..=7`.
    pub const EXEC_SOUND_CONSECUTIVE_START: u8 = 0xC8;
    pub const EXEC_SOUND_CONSECUTIVE_END: u8 = 0xCE;
    /// Stop all sound (speech continues).
    pub const STOP_ALL_SOUND_ALT: u8 = 0xCF;

    /// Execute speech string from buffer `N` only.
    pub const EXEC_SPEECH_INDIVIDUAL_START: u8 = 0xD0;
    pub const EXEC_SPEECH_INDIVIDUAL_END: u8 = 0xD7;
    /// Execute sound data from buffer `N` only.
    pub const EXEC_SOUND_INDIVIDUAL_START: u8 = 0xD8;
    pub const EXEC_SOUND_INDIVIDUAL_END: u8 = 0xDF;

    /// Execute allophone address stream from buffers `N..=7`.
    pub const EXEC_ALLOPHONE_CONSECUTIVE_START: u8 = 0xE0;
    pub const EXEC_ALLOPHONE_CONSECUTIVE_END: u8 = 0xE7;
    /// Execute register string from buffers `N..=7`.
    pub const EXEC_REGISTER_CONSECUTIVE_START: u8 = 0xE8;
    pub const EXEC_REGISTER_CONSECUTIVE_END: u8 = 0xEF;

    /// Execute allophone address stream from buffer `N` only.
    pub const EXEC_ALLOPHONE_INDIVIDUAL_START: u8 = 0xF0;
    pub const EXEC_ALLOPHONE_INDIVIDUAL_END: u8 = 0xF7;
    /// Execute register string from buffer `N` only.
    pub const EXEC_REGISTER_INDIVIDUAL_START: u8 = 0xF8;
    pub const EXEC_REGISTER_INDIVIDUAL_END: u8 = 0xFF;
}

/// Terminator bytes that end a buffer-RAM LOAD.
pub mod terminator {
    /// Ends a speech-string load (and the default-mode text-to-speech input).
    pub const SPEECH: u8 = 0x0D;
    /// Ends a sound-data, allophone, or register-string load.
    pub const SOUND: u8 = 0xFF;
}

/// Buffer RAM geometry: 8 buffers of 64 bytes, buffer `N` at
/// `N*BUFFER_SIZE..(N+1)*BUFFER_SIZE` of the firmware's buffer area.
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
    /// Bits 3-0 of a tone group's second byte: coarse tone period.
    pub const TONE_COARSE_MASK: u8 = 0x0F;
    /// Bit 7 of a noise group's second byte: "R" reuse-previous-amplitude flag.
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
