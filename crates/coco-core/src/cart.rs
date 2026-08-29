//! Cartridge port devices (FDC, ROM pack, Multi-Pak). See `DESIGN.md` §7.
//!
//! The cartridge in the port is stored as the closed [`Cart`] enum, not a
//! `Box<dyn Cartridge>` trait object: the cartridge set is in-crate and
//! finite, and an enum is what lets `SystemBus`/`Machine` derive serde for
//! save-states (`DESIGN.md` §9). The [`Cartridge`] trait remains as the
//! shared device interface each variant implements (and as the escape hatch
//! for out-of-crate test doubles through [`Cart::custom`]).

mod cart_enum;
mod empty;
mod gmc;
mod multipak;
mod rompak;

pub use cart_enum::Cart;
pub use empty::EmptySlot;
pub use gmc::GamesMasterCartridge;
pub use multipak::{MultiPak, mpi};
pub use rompak::{
    BANKED_PAK_MAX_LEN, BANKED_PAK_WINDOW_LEN, BankedPakError, BankedROMPak, ROM_PAK_MAX_LEN,
    ROMPak, ROMPakError,
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
    /// Read cartridge I/O: `$FF40–$FF5F` (the SCS* decode) plus `$FF60–$FF7E`,
    /// which some devices (Deluxe RS-232, Orchestra-90) decode off the raw
    /// address bus even though real hardware doesn't strobe SCS* there.
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
    /// Read the external ROM window (the CTS* decode): `$C000–$FDFF`, or all
    /// of `$8000–$FDFF` when INIT0 MC1:MC0 selects the 32K-external map.
    fn rom_read(&mut self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::rom_read`] for the debugger's
    /// disassembly/memory views. Default is open bus, so a device that can't
    /// read its ROM without side effects reports nothing rather than perturbing state.
    fn rom_peek(&self, _addr: u16) -> u8 {
        ROM_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::read`] (the SCS* I/O window).
    /// Defaults to open bus, since most cartridge I/O reads mutate device state.
    fn peek(&self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    /// Side-effect-free twin of [`Cartridge::control_read`] (`$FF7F`) for
    /// [`crate::SystemBus::peek`].
    fn peek_control(&self) -> u8 {
        IO_OPEN_BUS
    }
    /// True while this cartridge ties the expansion-port CART* line to the Q
    /// clock (~895 kHz): autostart paks do this so edges drive the PIA1 CB1
    /// FIRQ path and GIME EI0 continuously. Non-autostart paks leave this
    /// false, relying instead on BASIC's cold-start `'D'`,`'K'` probe.
    fn cart_line_ties_q(&self) -> bool {
        false
    }
    /// Current level of the CART* interrupt line as driven by the device
    /// (true = asserted, active-low). The level counterpart to
    /// [`Cartridge::cart_line_ties_q`]'s Q-burst; `SystemBus::poll_cart_interrupt`
    /// converts transitions into the PIA1 CB1 edge / GIME EI0 raise.
    fn cart_interrupt(&mut self) -> bool {
        false
    }
    /// Advance the cartridge's internal clocks by `cycles` CPU cycles. Called
    /// after every instruction, and once per burned cycle while the CPU is
    /// halted so a device such as the FDC can still pace its own work.
    fn tick(&mut self, _cycles: u32) {}
    /// Sample the cartridge's crystal-clocked sound generators, such as the
    /// GMC's SN76489A, over the next `dt` seconds of wall time, returning the
    /// (left, right) level pair, 0.0-1.0 per channel. Wall time rather than
    /// CPU cycles, since a cycle timebase would let the GIME double-speed
    /// poke retune the crystal. Mixed unconditionally: these carts drive
    /// their own outputs, not the mux-gated SND pin.
    fn generator_sample(&mut self, _dt: f64) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// True while the cartridge holds the CPU HALT* line low. Sampled at
    /// instruction boundaries.
    fn halt_asserted(&self) -> bool {
        false
    }
    /// Consume a pending NMI edge from the cartridge (the FD-502 gates the
    /// FDC's INTRQ onto the CPU NMI). Returns true at most once per edge.
    fn take_nmi(&mut self) -> bool {
        false
    }
    /// The cartridge's LATCHED (left, right) output levels, 0.0-1.0 per
    /// channel, such as the Orchestra-90's DAC pair. Unlike
    /// [`Cartridge::generator_sample`], these only change on a bus write, so
    /// the bus snapshots them into an [`crate::audio::AudioEvent`] instead of polling.
    fn sound_levels(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// Side-effect-free peek at whether an NMI edge is currently latched,
    /// without consuming it (unlike [`Cartridge::take_nmi`]).
    fn nmi_pending(&self) -> bool {
        false
    }
    /// Read the Multi-Pak Interface's own select register (`$FF7F`). Not
    /// routed through [`Cartridge::read`]/[`Cartridge::write`] (the SCS I/O
    /// window), since `$FF7F` must reach the MPI itself even when a plugged-in
    /// cart's own register decode could alias it. Default: open-bus/no-op.
    fn control_read(&mut self) -> u8 {
        IO_OPEN_BUS
    }
    /// Write the Multi-Pak Interface's own select register (`$FF7F`). See
    /// [`Cartridge::control_read`].
    fn control_write(&mut self, _val: u8) {}
    /// Re-run this cartridge's reset sequence (the CoCo's shared RESET*
    /// line). Default: nothing reset-sensitive to do.
    fn reset(&mut self) {}
    /// The cartridge's own analog audio output for this sample tick, in the
    /// same amplitude convention as [`Cartridge::sound_levels`]. Default:
    /// silent. Called once per `audio_sample` regardless of whether
    /// the sound mux currently selects the cartridge input, since some
    /// devices (the SSC's Sound Activity Circuit) must observe their own
    /// output continuously; `&mut self` because that observation is stateful
    /// (an envelope follower).
    fn audio_sample(&mut self) -> f32 {
        0.0
    }
    /// Restore-time fixups after a snapshot round-trip: rebuild any
    /// `#[serde(skip)]` construction-time scratch left at its `Default`.
    /// Default: nothing to rebuild.
    fn after_restore(&mut self) {}
    /// Restore-time payload-shape validation, checked before any media is
    /// reattached, against index/cursor fields the cartridge indexes its own
    /// buffers with — catches a bad one (from a hand-crafted payload) before
    /// Rust's bounds checks turn it into a panic. Default: nothing to check.
    fn validate_restored(&self) -> Result<(), String> {
        Ok(())
    }
}
