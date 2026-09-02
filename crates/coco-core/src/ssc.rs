//! Tandy Sound/Speech Cartridge (SSC, 26-3144): the `$FF7D`/`$FF7E` bus
//! handshake shell, an AY-3-8913 PSG and an SP0256-AL2 speech chip mixed
//! into the machine's audio output, and a host-byte protocol interpreter
//! standing in for the cartridge's TMS7040 firmware.
//!
//! **Not modelled** (see `docs/ssc-spec.md` "Deferred"): the TMS7040 CPU
//! that actually runs the cartridge's firmware, and with it the firmware's
//! ROM-based text-to-speech rules. What *is* modelled is the byte-stream
//! protocol that firmware speaks over `$FF7E`, per the Tandy Speech/Sound
//! Cartridge Owner's Manual (26-3144) Appendix A: command bytes that load
//! data into an 8×64-byte buffer RAM and execute it. Sound-data streams,
//! register-string streams, and direct AY register access (`$AF`) drive the
//! [`AY8913`] PSG; allophone streams drive the [`SP0256`] when its ROM is
//! installed ([`SoundSpeechCartridge::with_speech_rom`]). Speech-string
//! (ASCII text) commands are parsed to keep the state machine in sync but
//! their EXECUTE variants are no-ops: without the firmware there is no
//! text-to-allophone conversion to run.
//!
//! See [`dispatch_command`](SoundSpeechCartridge::dispatch_command) for the
//! top-level command dispatch in the [`protocol`] submodule. See
//! `docs/ssc-spec.md` for the full protocol writeup, including each judgment
//! call this implementation makes where the manual does not fully specify
//! behavior. The sound-data playback engine lives in the [`engine`]
//! submodule, allophone playback in [`speech`], and the Sound Activity
//! Circuit envelope follower in [`sac`].

use serde::{Deserialize, Serialize};

use crate::ay8913::AY8913;
use crate::cart::{Cartridge, IO_OPEN_BUS};
use crate::sp0256::{ROMSizeError, SP0256};

mod engine;
mod protocol;
mod sac;
mod speech;

pub use engine::{group, timing};
pub use protocol::{cmd, ram, terminator};
pub use speech::ALLOPHONE_COUNT;

use engine::Engine;
use protocol::{Mode, RAM_RESET_BYTE};
use speech::Speech;

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
    /// Bit 6: SP0256 SBY ("standby" = idle/ready), straight from the chip
    /// (MAME `m_spo->sby_r()`). Always set when no speech ROM is installed.
    pub const SPEECH_READY: u8 = 0x40;
    /// Bit 5: Sound Activity Circuit output, *inverted* — 1 = quiet, 0 =
    /// sound is playing (MAME returns `!sound_active`).
    pub const QUIET: u8 = 0x20;
}

/// Synthetic hold time for `busy` after a `$FF7E` write, in E-clock cycles.
///
/// On real hardware the TMS7040 firmware clears busy (through a port-bit toggle)
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

/// Speech level relative to the PSG on the cartridge's output: MAME routes
/// the SP0256 at 1.75 and the AY-3-8913 at 2.0 (`coco_ssc.cpp`
/// `SP0256_GAIN`/`AY8913_GAIN`).
const SPEECH_GAIN: f32 = 1.75 / 2.0;

/// The Sound/Speech Cartridge: `$FF7D`/`$FF7E` handshake, host-byte protocol
/// interpreter (see the module doc comment), and an AY-3-8913 plus optional
/// SP0256-AL2 mixed into the machine's audio output.
#[derive(Serialize, Deserialize)]
pub struct SoundSpeechCartridge {
    ay: AY8913,
    /// The speech chip, present only when its AL2 ROM was supplied. Its ROM
    /// is not snapshotted; see [`SoundSpeechCartridge::attach_speech_rom`].
    /// `#[serde(default)]` (snapshot evolution rule 2, [`crate::snapshot`]):
    /// a pre-field snapshot restores chip-less, exactly the old behaviour.
    #[serde(default)]
    sp0256: Option<SP0256>,
    /// Bit 0 of the last byte written to `$FF7D`, for falling-edge detection
    /// on the next write. Power-on-reset starts clear so the very first
    /// `$FF7D` write (even if it's bit0=0) is never itself treated as a
    /// falling edge — only a later 1-then-0 pair is (MAME
    /// `coco_ssc_device::device_reset` primes `m_reset_line` the same way).
    prev_reset_bit0: bool,
    /// Last byte latched from a `$FF7E` write (the "Port A latch" the real
    /// TMS7040 firmware reads and interprets as a command/data byte).
    /// Stored for tests/debug; the actual interpretation happens in
    /// [`SoundSpeechCartridge::dispatch`], invoked synchronously from
    /// [`SoundSpeechCartridge::write_data`].
    host_latch: u8,
    busy: bool,
    /// E-clock cycles remaining before [`SoundSpeechCartridge::busy`] synthetically clears —
    /// see [`BUSY_HOLD_CYCLES`].
    busy_countdown: u32,
    // Sound Activity Circuit state (see the [`sac`] module doc comment).
    sac_hpf_prev_in: f32,
    sac_hpf_prev_out: f32,
    sac_envelope: f32,
    /// True while the SAC considers the cartridge's own output "playing"
    /// (drives `$FF7E` bit 5, inverted).
    sac_sound_active: bool,

    // ---- Host byte protocol state (see the module doc comment) ----------
    /// Flat 8×64-byte buffer RAM — see the [`ram`] module doc comment.
    /// `ram::SIZE` (512) exceeds serde's built-in array impl ceiling (32),
    /// hence the small helper module.
    #[serde(with = "crate::serde_util::byte_array")]
    ram: [u8; ram::SIZE],
    /// Top-level protocol dispatch state — see [`Mode`].
    mode: Mode,
    /// `$8F`'s postbyte: scales every subsequent sound-data event's
    /// duration — see [`timing`].
    timer_base: u8,
    /// The sequential sound-data playback engine — see [`Engine`].
    engine: Engine,
    /// The allophone-stream cursor — see [`Speech`]. `#[serde(default)]`
    /// as for `sp0256`: a pre-field snapshot restores with no stream active.
    #[serde(default)]
    speech: Speech,
}

impl Default for SoundSpeechCartridge {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundSpeechCartridge {
    /// A cartridge with no speech ROM: the AY half works, speech stays silent
    /// and [`status::SPEECH_READY`] always reads set.
    pub fn new() -> Self {
        Self {
            ay: AY8913::new(),
            sp0256: None,
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
            speech: Speech::default(),
        }
    }

    /// A cartridge with an SP0256-AL2 fitted, given its 2 KB allophone ROM.
    pub fn with_speech_rom(rom: &[u8]) -> Result<Self, ROMSizeError> {
        let mut ssc = Self::new();
        ssc.attach_speech_rom(rom)?;
        Ok(ssc)
    }

    /// Fit (or, after a snapshot restore, re-supply the ROM of) the SP0256.
    /// A restored chip keeps its sequencer state; a fresh one starts in reset.
    pub fn attach_speech_rom(&mut self, rom: &[u8]) -> Result<(), ROMSizeError> {
        match &mut self.sp0256 {
            Some(chip) => chip.reattach_rom(rom),
            None => {
                self.sp0256 = Some(SP0256::new(rom)?);
                Ok(())
            }
        }
    }

    /// Whether an SP0256 is fitted (with or without its ROM reattached).
    pub fn has_speech_chip(&self) -> bool {
        self.sp0256.is_some()
    }

    /// The speech chip, for tests/debugging.
    pub fn sp0256(&self) -> Option<&SP0256> {
        self.sp0256.as_ref()
    }

    /// Direct AY-3-8913 register write, bypassing the host-byte protocol
    /// (used internally by the protocol interpreter, and for tests/debugging).
    pub fn ay_write(&mut self, reg: u8, val: u8) {
        self.ay.write_reg(reg, val);
    }

    /// Direct AY-3-8913 register read (see [`SoundSpeechCartridge::ay_write`]).
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
        if bit0 {
            // Bit 0 is the SP0256's RESET pin (MAME: every write with it set
            // resets the chip, not just edges).
            if let Some(chip) = &mut self.sp0256 {
                chip.reset();
            }
        }
        if falling_edge {
            // Falling edge also resets the AY and readies the firmware for a
            // new command (MAME coco_ssc_device).
            self.ay.reset();
            self.busy = false;
            self.busy_countdown = 0;
            self.reset_protocol_state();
        }
    }

    /// Resets buffer RAM, dispatch mode, timer base, and both playback
    /// engines — shared by [`SoundSpeechCartridge::write_reset`] and
    /// [`Cartridge::reset`]. Does NOT touch AY, SP0256, busy handshake, or
    /// SAC state.
    fn reset_protocol_state(&mut self) {
        self.ram = [RAM_RESET_BYTE; ram::SIZE];
        self.mode = Mode::Idle;
        self.timer_base = timing::DEFAULT_TIMER_BASE;
        self.engine = Engine::default();
        self.speech = Speech::default();
    }

    /// `$FF7E` write: entry point for the host-byte protocol. Per the manual
    /// (p.10), a byte written while [`SoundSpeechCartridge::busy`] is set is
    /// discarded — not latched, not processed, and doesn't restart the busy window.
    fn write_data(&mut self, val: u8) {
        if self.busy {
            return;
        }
        self.host_latch = val;
        self.busy = true;
        self.busy_countdown = BUSY_HOLD_CYCLES;
        // Real hardware asserts TMS7000 INT3 here; not modelled — we
        // interpret the byte synchronously instead.
        self.dispatch(val);
    }

    fn read_data(&self) -> u8 {
        let mut s = status::BASE;
        if !self.busy {
            s |= status::NOT_BUSY;
        }
        if self.sp0256.as_ref().is_none_or(SP0256::sby) {
            s |= status::SPEECH_READY;
        }
        if !self.sac_sound_active {
            s |= status::QUIET;
        }
        s
    }
}

impl Cartridge for SoundSpeechCartridge {
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
        self.feed_speech();
        if let Some(chip) = &mut self.sp0256 {
            chip.step(cycles);
        }
    }

    fn reset(&mut self) {
        self.ay.reset();
        if let Some(chip) = &mut self.sp0256 {
            chip.reset();
        }
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

    /// Drains the AY's output and feeds the Sound Activity Circuit
    /// unconditionally — `$FF7E` bit 5 must reflect the cartridge's own
    /// output regardless of whether the sound mux is selecting it — then
    /// adds the speech chip, which bypasses the SAC as on MAME's board.
    fn audio_sample(&mut self) -> f32 {
        let psg = self.ay.drain();
        self.update_sac(psg);
        let speech = self.sp0256.as_ref().map_or(0.0, SP0256::output);
        psg + SPEECH_GAIN * speech
    }

    /// Rebuilds `ay.dac` — pure construction-time scratch, skipped from the
    /// snapshot (`Ay8913::after_restore`).
    fn after_restore(&mut self) {
        self.ay.after_restore();
    }

    /// Restore-only: rejects a mid buffer-RAM-load, mid sound-engine, or mid
    /// allophone-stream snapshot whose `cursor`/`cap` don't satisfy `cursor
    /// <= cap <= ram::SIZE` — all three index `self.ram` with no bounds
    /// check of their own.
    fn validate_restored(&self) -> Result<(), String> {
        if let Mode::Loading(load) = &self.mode {
            check_ram_cursor_cap("Load", load.cursor, load.cap)?;
        }
        // An inactive cursor's cursor/cap are never read, so only check while active.
        if self.engine.active {
            check_ram_cursor_cap("Engine", self.engine.cursor, self.engine.cap)?;
        }
        if self.speech.active {
            check_ram_cursor_cap("Speech", self.speech.cursor, self.speech.cap)?;
        }
        Ok(())
    }
}

/// Shared bound check for [`SoundSpeechCartridge::validate_restored`]'s cursor/cap pairs
/// ([`Load`]/[`Engine`]/[`Speech`]): `cursor <= cap <= ram::SIZE`.
fn check_ram_cursor_cap(name: &str, cursor: usize, cap: usize) -> Result<(), String> {
    if cap > ram::SIZE {
        return Err(format!(
            "SSC: {name}.cap ({cap}) exceeds ram::SIZE ({})",
            ram::SIZE
        ));
    }
    if cursor > cap {
        return Err(format!(
            "SSC: {name}.cursor ({cursor}) exceeds {name}.cap ({cap})"
        ));
    }
    Ok(())
}
