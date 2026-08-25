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
//! functional and drive the [`AY8913`] PSG. Speech, allophone, and SP0256
//! command bytes are parsed just enough to keep the state machine in sync
//! (their LOAD variants still fill the buffer RAM per the flat-RAM model
//! below) but their EXECUTE variants are no-ops — no SP0256 is emulated, so
//! there is nothing to make them audible.
//!
//! See [`dispatch_command`](SoundSpeechCartridge::dispatch_command) for the top-level command
//! dispatch (in the [`protocol`] submodule) and `docs/ssc-spec.md` for the
//! full protocol writeup, including every judgment call this implementation
//! had to make where the manual doesn't fully specify behavior. The
//! sound-data playback engine lives in the [`engine`] submodule, and the
//! Sound Activity Circuit envelope follower lives in the [`sac`] submodule.

use serde::{Deserialize, Serialize};

use crate::ay8913::AY8913;
use crate::cart::{Cartridge, IO_OPEN_BUS};

mod engine;
mod protocol;
mod sac;

pub use engine::{group, timing};
pub use protocol::{cmd, ram, terminator};

use engine::Engine;
use protocol::{Mode, RAM_RESET_BYTE};

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
    /// this is always set — see [`super::SoundSpeechCartridge`]'s doc comment. The
    /// execute-speech commands are no-ops by design and never clear this
    /// bit, since there is no speech synthesis running to report busy.
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

/// The Sound/Speech Cartridge: `$FF7D`/`$FF7E` handshake, host-byte protocol
/// interpreter (see the module doc comment), and an AY-3-8913 mixed into the
/// machine's audio output.
///
/// No SP0256 speech synthesizer is modelled, so [`status::SPEECH_READY`] is
/// always reported set (idle/ready) — see the module doc comment.
#[derive(Serialize, Deserialize)]
pub struct SoundSpeechCartridge {
    ay: AY8913,
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
}

impl Default for SoundSpeechCartridge {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundSpeechCartridge {
    pub fn new() -> Self {
        Self {
            ay: AY8913::new(),
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
        if falling_edge {
            // Falling edge also resets the AY and readies the firmware for a
            // new command (MAME coco_ssc_device).
            self.ay.reset();
            self.busy = false;
            self.busy_countdown = 0;
            self.reset_protocol_state();
        }
        // bit0=1 alone: asserts the SP0256 RESET pin, which isn't emulated — no-op.
    }

    /// Resets buffer RAM, dispatch mode, timer base, and the sound engine —
    /// shared by [`SoundSpeechCartridge::write_reset`] and [`Cartridge::reset`].
    /// Does NOT touch AY, busy handshake, or SAC state.
    fn reset_protocol_state(&mut self) {
        self.ram = [RAM_RESET_BYTE; ram::SIZE];
        self.mode = Mode::Idle;
        self.timer_base = timing::DEFAULT_TIMER_BASE;
        self.engine = Engine::default();
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
        let mut s = status::BASE | status::SPEECH_READY;
        if !self.busy {
            s |= status::NOT_BUSY;
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

    /// Drains the AY's output and feeds the Sound Activity Circuit
    /// unconditionally — `$FF7E` bit 5 must reflect the cartridge's own
    /// output regardless of whether the sound mux is selecting it.
    fn audio_sample(&mut self) -> f32 {
        let out = self.ay.drain();
        self.update_sac(out);
        out
    }

    /// Rebuilds `ay.dac` — pure construction-time scratch, skipped from the
    /// snapshot (`Ay8913::after_restore`).
    fn after_restore(&mut self) {
        self.ay.after_restore();
    }

    /// Restore-only: rejects a mid buffer-RAM-load or mid sound-engine
    /// snapshot whose `cursor`/`cap` don't satisfy `cursor <= cap <=
    /// ram::SIZE` — both index `self.ram` with no bounds check of their own.
    fn validate_restored(&self) -> Result<(), String> {
        if let Mode::Loading(load) = &self.mode {
            check_ram_cursor_cap("Load", load.cursor, load.cap)?;
        }
        // An inactive engine's cursor/cap are never read, so only check while active.
        if self.engine.active {
            check_ram_cursor_cap("Engine", self.engine.cursor, self.engine.cap)?;
        }
        Ok(())
    }
}

/// Shared bound check for [`SoundSpeechCartridge::validate_restored`]'s two cursor/cap pairs
/// ([`Load`]/[`Engine`]): `cursor <= cap <= ram::SIZE`.
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
