//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! The cartridge in the port is stored as the closed [`Cart`] enum, not a
//! `Box<dyn Cartridge>` trait object: the cartridge set is in-crate and
//! finite, and an enum is what lets `SystemBus`/`Machine` derive serde for
//! save-states (`DESIGN.md` §9). The [`Cartridge`] trait remains as the
//! shared device interface each variant implements (and as the escape hatch
//! for out-of-crate test doubles via [`Cart::custom`]).

mod cart_enum;
mod empty;
mod gmc;
mod multipak;
mod rompak;

pub use cart_enum::Cart;
pub use empty::EmptySlot;
pub use gmc::GamesMasterCartridge;
pub use multipak::{mpi, MultiPak};
pub use rompak::{
    BankedPakError, BankedROMPak, ROMPak, RomPakError, BANKED_PAK_MAX_LEN, BANKED_PAK_WINDOW_LEN,
    ROM_PAK_MAX_LEN,
};

/// Value read from the external ROM window when nothing drives the bus:
/// $00, matching real hardware / MAME coco3 (verified by MAME trace-diff,
/// 2026-07-02 — an empty slot's `LDD $C000` yields $0000, not $FFFF).
pub const ROM_OPEN_BUS: u8 = 0x00;

/// Value read from the cartridge I/O window ($FF40–$FF5F, SCS*) when nothing
/// drives the bus: floats high, like an unstrobed PIA input pin.
pub const IO_OPEN_BUS: u8 = 0xFF;

/// A device on the cartridge port.
pub trait Cartridge {
    /// Read cartridge I/O: `$FF40–$FF5F` (the SCS* decode) plus
    /// `$FF60–$FF7E`, which real hardware doesn't strobe with SCS* but
    /// devices there (Deluxe RS-232, Orchestra-90) decode off the raw
    /// address bus — the port carries all 16 address lines.
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::rom_read`] for the debugger's
    /// disassembly/memory views ([`crate::SystemBus::peek`]). Overridden by
    /// cartridges whose ROM read is a pure array fetch (ROM paks, the FD-502
    /// controller ROM); the default is open bus so a device that can't read
    /// its ROM without side effects safely reports nothing rather than
    /// perturbing state.
    fn rom_peek(&self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::read`] (the SCS* I/O window) for
    /// [`crate::SystemBus::peek`]. Defaults to open bus: most cartridge I/O
    /// reads (FDC status, etc.) mutate device state, so the debugger reports
    /// open bus rather than risk a side effect.
    fn peek(&self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::control_read`] (`$FF7F`) for
    /// [`crate::SystemBus::peek`].
    fn peek_control(&self) -> u8 {
        IO_OPEN_BUS
    }
    /// True while this cartridge ties the expansion-port CART* line to the Q
    /// clock (~895 kHz): auto-start game paks do this so edges arrive
    /// continuously, which drives both the legacy PIA1 CB1 FIRQ path and the
    /// GIME EI0 input the same physical pin feeds. `SystemBus::hsync` polls
    /// this once per scanline — plenty to model a ~895 kHz signal, since the
    /// PIA/GIME only care that *an* edge keeps arriving. Disk-BASIC-style
    /// paks (no autostart) leave this false; they rely on BASIC's cold-start
    /// probe of `$C000`/`$C001` for `'D'`,`'K'` instead.
    fn cart_line_ties_q(&self) -> bool {
        false
    }
    /// Current level of the CART* interrupt line as driven by the device:
    /// true = asserted (the active-low pin held low). The level counterpart
    /// to [`Cartridge::cart_line_ties_q`]'s Q-burst: a cartridge with a real
    /// interrupt source — the Deluxe RS-232's 6551 ACIA IRQ output — holds
    /// this while its interrupt condition stands, and
    /// `SystemBus::poll_cart_interrupt` converts the transitions into the
    /// PIA1 CB1 edge and GIME EI0 raise that the shared physical pin feeds.
    /// Default: never asserted.
    fn cart_interrupt(&mut self) -> bool {
        false
    }
    /// Advance the cartridge's internal clocks by `cycles` CPU cycles. Called
    /// by the machine loop after every instruction (and once per burned cycle
    /// while the CPU is halted, so a device can pace work — the FDC's DRQ
    /// cadence — while it holds the HALT line).
    fn tick(&mut self, _cycles: u32) {}
    /// Sample the cartridge's crystal-clocked sound generators (the GMC's
    /// SN76489A) over the next `dt` seconds of wall time, returning the
    /// (left, right) level pair, 0.0–1.0 per channel. Wall time — not CPU
    /// cycles — because these chips run off their own crystal: a CPU-cycle
    /// timebase would let the GIME double-speed poke retune them. Called
    /// once per audio grid slot ([`crate::audio::OVERSAMPLE`] per scanline)
    /// and mixed unconditionally: such carts drive their own outputs, not
    /// the mux-gated SND pin.
    fn generator_sample(&mut self, _dt: f64) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// True while the cartridge holds the CPU HALT* line low (the FD-502's
    /// transfer handshake). Sampled at instruction boundaries.
    fn halt_asserted(&self) -> bool {
        false
    }
    /// Consume a pending NMI edge from the cartridge (the FD-502 gates the
    /// FDC's INTRQ onto the CPU NMI). Returns true at most once per edge.
    fn take_nmi(&mut self) -> bool {
        false
    }
    /// The cartridge's LATCHED (left, right) output levels, 0.0–1.0 per
    /// channel — the Orchestra-90's write-only DAC pair. Unlike
    /// [`Cartridge::generator_sample`] these only change on a bus write, so
    /// the bus snapshots them into a timestamped [`crate::audio::AudioEvent`]
    /// after every cartridge-window write instead of polling; between writes
    /// the level holds exactly. Mixed unconditionally (own outputs, not the
    /// SND pin).
    fn sound_levels(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// Side-effect-free peek at whether an NMI edge is currently latched,
    /// without consuming it (unlike [`Cartridge::take_nmi`]) — the
    /// debugger's hardware-state panel uses this to show the cart port's NMI
    /// line without perturbing the pending edge a running program still
    /// needs to see (`docs/plan-debugger.md` §3, "cart line states").
    fn nmi_pending(&self) -> bool {
        false
    }
    /// Read the Multi-Pak Interface's own select register (`$FF7F`). Not
    /// routed through [`Cartridge::read`]/[`Cartridge::write`]: those carry
    /// the SCS I/O window ($FF40-$FF5F), and `$FF7F` must reach the MPI
    /// itself even when a `DiskCart` (whose own register decode could alias
    /// it) is plugged into the MPI's selected SCS slot. Every cartridge other
    /// than [`MultiPak`] leaves this at the default open-bus/no-op.
    fn control_read(&mut self) -> u8 {
        IO_OPEN_BUS
    }
    /// Write the Multi-Pak Interface's own select register (`$FF7F`). See
    /// [`Cartridge::control_read`].
    fn control_write(&mut self, _val: u8) {}
    /// Re-run this cartridge's reset sequence (the CoCo's RESET* line, which
    /// the expansion port shares). [`MultiPak`] reloads its select register
    /// from the front-panel switch and forwards the reset to all 4 slots;
    /// every other cartridge has nothing reset-sensitive to do.
    fn reset(&mut self) {}
    /// The cartridge's own analog audio output for this sample tick, in the
    /// same amplitude convention as
    /// [`SystemBus::sound_sample`](crate::bus::SystemBus::sound_sample) (0.0
    /// silence, full scale comparable to that method's other sources).
    /// Default: silent — most cartridges (ROM paks, the FD-502, the VHD
    /// interface) have no audio output of their own. The Sound/Speech
    /// Cartridge ([`crate::ssc::SoundSpeechCartridge`]) is the one device that overrides
    /// this.
    ///
    /// Called exactly once per `sound_sample`, regardless of whether the
    /// CoCo's sound mux currently selects the cartridge input: some devices
    /// (the SSC's Sound Activity Circuit) need to observe their own output
    /// continuously, independent of what's actually reaching the speaker.
    /// Takes `&mut self` because that continuous observation is itself
    /// stateful (an envelope follower).
    fn audio_sample(&mut self) -> f32 {
        0.0
    }
    /// Restore-time fixups after a snapshot round-trip
    /// (`docs/plan-save-states.md`): rebuild any `#[serde(skip)]`
    /// construction-time scratch (lookup tables, etc.) that this cartridge's
    /// own `Serialize`/`Deserialize` impl left at its `Default`. Default:
    /// nothing to rebuild — most cartridges have no such scratch.
    fn after_restore(&mut self) {}
    /// Restore-time payload-shape validation (`docs/plan-save-states.md`):
    /// checked once by [`crate::snapshot::restore::validate_payload_shape`], before
    /// any media is reattached, against index/cursor/cap-style deserialized
    /// fields this cartridge indexes its own buffers with — Rust's own
    /// bounds checks turn a bad one (from a hand-crafted payload) into a
    /// panic, not a graceful error, unless this catches it first. See
    /// `crate::ssc::Ssc`/`crate::fdc::DiskCart`'s overrides. Default:
    /// nothing to check.
    fn validate_restored(&self) -> Result<(), String> {
        Ok(())
    }
}
