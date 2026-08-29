//! 6551 ACIA (MOS 6551 / Rockwell R6551 / WDC W65C51 — not a Motorola
//! MC-prefixed part), the UART at the heart of the Tandy Deluxe RS-232
//! Program Pak. MAME's `src/devices/machine/mos6551.cpp` on the `master`
//! branch defines the register and bit semantics. Task 2 checked every fact
//! that follows against that source rather than deriving it from a datasheet.
//!
//! # Byte-level timing divergence
//!
//! MAME's `mos6551_device` is a bit-serial engine: it shifts one bit at a
//! time off a per-bit timer and can therefore generate real parity/framing
//! errors and expose bit-accurate RS-232 waveforms. This model is
//! deliberately **byte-level**: [`ACIA6551::tick`] runs a whole-frame timer
//! for the receiver and transmitter, sized from the same baud-rate math MAME
//! uses (see `BAUD_DIVIDER`), and delivers/consumes a complete byte when
//! that timer expires. Consequences of the divergence, called out again at
//! each relevant spot that follows:
//! - Parity and framing errors are never generated *internally* (there is no
//!   bit shifter to mis-sample); the status bits exist for completeness and
//!   are cleared exactly where MAME clears them, but this model never sets
//!   them.
//! - Echo mode ([`command::ECHO`]) is approximated at the byte boundary:
//!   MAME echoes bit-by-bit as they're shifted in; this model queues the
//!   whole received byte onto the transmit wire the instant the RX frame
//!   completes. It also skips MAME's nuance of forcing the echoed output to
//!   mark while overrun is set.
//! - The 1.5-stop-bit case for 5-bit words is collapsed to 2 stop bits (see
//!   `ACIA6551::stop_bits`).
//! - The model checks DCD/DSR level-change IRQ arming once per [`ACIA6551::tick`]
//!   call rather than on a live edge — MAME itself ties this to the receive
//!   clock and carries `TODO` comments that acknowledge unresolved timing.
//!   The model ties the check to its own tick boundary. See
//!   `ACIA6551::tick_modem_lines`.
//!
//! # Wire interface
//!
//! This module is a pure chip model with a byte-level wire interface: no
//! knowledge of hosts, sockets, or files. [`ACIA6551::take_tx_byte`] /
//! [`ACIA6551::receive_byte`] / [`ACIA6551::rx_ready`] / [`ACIA6551::set_dcd`]
//! / [`ACIA6551::set_dsr`] are the seam for a future cartridge-glue layer to
//! drive from a real host endpoint.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

mod frame;
mod irq;
mod registers;

/// CPU clock at normal speed (`crate::CPU_HZ` in `lib.rs` — NTSC crystal /32,
/// MAME `coco3.cpp`). Duplicated here as a private constant because
/// `crate::CPU_HZ` is not `pub`; the value must stay in sync with `lib.rs`.
const CPU_HZ: u64 = 894_886;

/// ACIA baud-rate-generator crystal: 1.8432 MHz, the standard reference for
/// 6551-family parts (MAME `mos6551.cpp`: `baudrate = clock() / 16 /
/// divider`, with `clock()` fixed at this value regardless of host CPU
/// speed).
const ACIA_CRYSTAL_HZ: u64 = 1_843_200;

/// The baud-rate generator divides [`ACIA_CRYSTAL_HZ`] by this fixed factor
/// before applying the per-baud-index [`BAUD_DIVIDER`] (MAME `mos6551.cpp`).
const BAUD_CLOCK_DIVISOR: u64 = 16;

/// Every frame has exactly one start bit.
const START_BITS: u32 = 1;

/// Baud-rate divider, indexed by control register bits 3:0. `baud =
/// `[`ACIA_CRYSTAL_HZ`]` / `[`BAUD_CLOCK_DIVISOR`]` / BAUD_DIVIDER[index]`
/// (MAME `mos6551.cpp`, `internal_registers` divisor table). Index 0 is
/// nominally "external clock"; MAME's own table uses a divider of 1 there
/// and this model does the same rather than inventing external-clock
/// behavior.
const BAUD_DIVIDER: [u32; 16] = [
    1, 2304, 1536, 1048, 856, 768, 384, 192, 96, 64, 48, 32, 24, 16, 12, 6,
];

/// Status register bits (offset 1, read-only view; see [`ACIA6551::read`]).
pub mod status {
    /// Bit 0: parity error. This model never sets it (see module doc,
    /// "byte-level timing divergence") but clears it exactly where MAME
    /// clears it (RDR read, programmed reset).
    pub const PARITY_ERROR: u8 = 0x01;
    /// Bit 1: framing error. Never set by this model; see [`PARITY_ERROR`].
    pub const FRAMING_ERROR: u8 = 0x02;
    /// Bit 2: overrun — a completed RX frame found RDRF already set (the
    /// previous byte hadn't been read yet).
    pub const OVERRUN: u8 = 0x04;
    /// Bit 3: Receive Data Register Full — a byte is waiting in RDR.
    pub const RDRF: u8 = 0x08;
    /// Bit 4: Transmit Data Register Empty — TDR is free to accept a new
    /// byte. MAME additionally masks this bit with the CTS input on status
    /// read; this model has no CTS input (treated as permanently asserted),
    /// so no masking is applied here.
    pub const TDRE: u8 = 0x10;
    /// Bit 5: DCD (carrier detect), live level — tracks the input
    /// unconditionally, independent of DTR (only the *IRQ arming* on a DCD
    /// change is gated by DTR; see [`crate::acia6551::command::DTR`]).
    pub const DCD: u8 = 0x20;
    /// Bit 6: DSR (data set ready), live level — same independence from DTR
    /// as [`DCD`].
    pub const DSR: u8 = 0x40;
    /// Bit 7: IRQ output — set whenever any IRQ source bit is armed; cleared
    /// (along with every armed source) by a status register read.
    pub const IRQ: u8 = 0x80;
}

/// Command register bits (offset 2, read/write).
pub mod command {
    /// Bit 0: DTR enable. Gates rx-IRQ, tx-IRQ, and DCD/DSR IRQ arming, and
    /// idles the transmitter — a written TDR remains pending, TDRE stays clear,
    /// nothing transmits (MAME `mos6551.cpp` `write_command`/`update_irq`).
    pub const DTR: u8 = 0x01;
    /// Bit 1: receiver IRQ **disable** — 0 = rx-IRQ enabled, 1 = disabled
    /// (inverted sense, matching the real 6551 and MAME's field name).
    pub const RX_IRQ_DISABLE: u8 = 0x02;
    /// Bits 3:2: transmitter control field — mask before shifting by
    /// [`TX_CONTROL_SHIFT`]. See [`crate::acia6551::tx_control`] for the
    /// four encoded values.
    pub const TX_CONTROL_MASK: u8 = 0x0C;
    /// Shift to bring [`TX_CONTROL_MASK`] down to a 0..=3 value.
    pub const TX_CONTROL_SHIFT: u8 = 2;
    /// Bit 4: echo mode — see the module doc's "byte-level timing
    /// divergence" note on how this model approximates it.
    pub const ECHO: u8 = 0x10;
    /// Bits 7:5: parity mode — mask before shifting by [`PARITY_SHIFT`].
    /// Even values (0/2/4/6) mean parity disabled; odd values mean enabled:
    /// 1 = odd, 3 = even, 5 = mark, 7 = space. At byte level only "enabled
    /// or not" matters (it adds one bit to the frame; see
    /// `ACIA6551::frame_bits`) — this model does not distinguish which
    /// parity mode, since it never generates or checks parity bits.
    pub const PARITY_MASK: u8 = 0xE0;
    /// Shift to bring [`PARITY_MASK`] down to a 0..=7 value.
    pub const PARITY_SHIFT: u8 = 5;
}

/// Decoded values of `(command & `[`command::TX_CONTROL_MASK`]`) >>
/// `[`command::TX_CONTROL_SHIFT`]` (MAME `mos6551.cpp` `write_command`).
pub mod tx_control {
    /// RTS output off, no TDRE IRQ. The transmitter itself still runs —
    /// MAME's table only encodes {tx-IRQ, RTS, BREAK}; only [`BREAK`] (or
    /// DTR disabled) stops transmission.
    pub const RTS_OFF: u8 = 0;
    /// Transmitter on, RTS output on, TDRE IRQ enabled.
    pub const IRQ_ENABLED: u8 = 1;
    /// Transmitter on, RTS output on, TDRE IRQ disabled.
    pub const RTS_ON: u8 = 2;
    /// Transmitter forced to BREAK, RTS output on, TDRE IRQ disabled. While
    /// in this state TDR is never consumed and no TDRE IRQ ever fires.
    pub const BREAK: u8 = 3;
}

/// Control register bits (offset 3, read/write). None of the bits here
/// generate errors at the byte level (see module doc); they only feed
/// `ACIA6551::cycles_per_frame`.
pub mod control {
    /// Bits 3:0: baud-rate index — see `BAUD_DIVIDER`.
    pub const BAUD_MASK: u8 = 0x0F;
    /// Bit 4: receiver clock source. Stored for completeness; this model
    /// has no external-clock behavior to switch (see `BAUD_DIVIDER`'s
    /// index-0 note).
    pub const RX_CLOCK_SOURCE: u8 = 0x10;
    /// Bits 6:5: word length field — mask before shifting by
    /// [`WORD_LENGTH_SHIFT`]; `word_length = 8 - field`.
    pub const WORD_LENGTH_MASK: u8 = 0x60;
    /// Shift to bring [`WORD_LENGTH_MASK`] down to a 0..=3 value.
    pub const WORD_LENGTH_SHIFT: u8 = 5;
    /// Bit 7: 2 stop bits instead of 1 (also covers the 5-bit-word 1.5-stop
    /// case, collapsed to 2 — see `ACIA6551::stop_bits`).
    pub const STOP_BITS_2: u8 = 0x80;
}

/// Internal IRQ-source bitmask (not a register — MAME `mos6551_device`
/// tracks these as separate `m_irq_state`-style booleans; here they're bits
/// of one byte for compactness). The IRQ output ([`status::IRQ`]) is the
/// OR of all of these; a status register read clears the whole mask at
/// once (MAME `mos6551.cpp` `read_status_register`).
mod irq_source {
    pub const DCD: u8 = 0x01;
    pub const DSR: u8 = 0x02;
    pub const RDRF: u8 = 0x04;
    pub const TDRE: u8 = 0x08;
}

/// A 6551 ACIA: registers, IRQ-source tracking, and a byte-level RX/TX frame
/// timer. See the module doc for the MAME source and the deliberate
/// byte-level timing divergence.
#[derive(Serialize, Deserialize)]
pub struct ACIA6551 {
    /// Receive Data Register — last completed RX byte.
    rdr: u8,
    /// Transmit Data Register — last byte written by the CPU, pending
    /// consumption by the transmitter.
    tdr: u8,
    /// Command register (offset 2).
    command: u8,
    /// Control register (offset 3).
    control: u8,
    /// Status register (offset 1) as it stands, including the
    /// live IRQ bit — kept in sync by [`Self::update_irq_output`] whenever
    /// `irq_sources` changes.
    status: u8,
    /// Bitmask of armed IRQ sources (see [`irq_source`]).
    irq_sources: u8,

    /// Live DCD input level (true = carrier present).
    dcd_level: bool,
    /// DCD level as observed at the last [`Self::tick_modem_lines`] check —
    /// used only to detect a change since the previous tick (see module
    /// doc's "byte-level timing divergence" on DCD/DSR IRQ timing).
    dcd_checked: bool,
    /// Live DSR input level.
    dsr_level: bool,
    /// DSR level as observed at the last tick check; see `dcd_checked`.
    dsr_checked: bool,

    /// RTS output state, derived from the transmitter-control field on every
    /// command write. Tracked for a future host wire — no external effect
    /// yet.
    rts: bool,

    /// Byte latched out of TDR at transmit-start, shifting for the duration
    /// of `tx_timer`.
    tx_shift_byte: u8,
    /// CPU cycles remaining until the in-progress TX frame completes, or
    /// `None` if the transmitter is idle.
    tx_timer: Option<u32>,
    /// Completed TX bytes waiting to be drained by [`Self::take_tx_byte`].
    /// A queue (rather than a single slot) because a real TX frame
    /// completion and an echoed RX byte can both complete in the same
    /// [`Self::tick`] call.
    tx_output: VecDeque<u8>,

    /// Byte handed to [`Self::receive_byte`], landing in RDR when
    /// `rx_timer` expires.
    rx_pending_byte: u8,
    /// CPU cycles remaining until the in-progress RX frame completes, or
    /// `None` if the receiver is idle ([`Self::rx_ready`]).
    rx_timer: Option<u32>,
}

impl Default for ACIA6551 {
    fn default() -> Self {
        Self::new()
    }
}

impl ACIA6551 {
    /// Power-on state: identical to [`Self::hardware_reset`] (MAME
    /// `mos6551_device::device_reset` runs on both power-on and RESET*).
    pub fn new() -> Self {
        let mut acia = Self {
            rdr: 0,
            tdr: 0,
            command: 0,
            control: 0,
            status: 0,
            irq_sources: 0,
            dcd_level: false,
            dcd_checked: false,
            dsr_level: false,
            dsr_checked: false,
            rts: false,
            tx_shift_byte: 0,
            tx_timer: None,
            tx_output: VecDeque::new(),
            rx_pending_byte: 0,
            rx_timer: None,
        };
        acia.hardware_reset();
        acia
    }

    /// RESET* pin: status becomes TDRE plus live DCD/DSR bits; command,
    /// control, and IRQ sources clear; frame timers idle (MAME
    /// `mos6551_device::device_reset`).
    pub fn hardware_reset(&mut self) {
        self.rdr = 0;
        self.tdr = 0;
        self.command = 0;
        self.control = 0;
        self.irq_sources = 0;
        self.tx_timer = None;
        self.rx_timer = None;
        self.tx_output.clear();
        self.tx_shift_byte = 0;
        self.rx_pending_byte = 0;
        self.rts = false;

        self.status = status::TDRE;
        if self.dcd_level {
            self.status |= status::DCD;
        }
        if self.dsr_level {
            self.status |= status::DSR;
        }
        // Re-baseline so the reset itself isn't seen as a DCD/DSR edge.
        self.dcd_checked = self.dcd_level;
        self.dsr_checked = self.dsr_level;
    }

    /// Register read, `reg` 0-3 = `$FF68`-`$FF6B` offsets.
    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 0x03 {
            0 => self.read_rdr(),
            1 => self.read_status(),
            2 => self.command,
            3 => self.control,
            _ => unreachable!("reg & 0x03 is always 0..=3"),
        }
    }

    /// Register write, `reg` 0-3 = `$FF68`-`$FF6B` offsets.
    pub fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x03 {
            0 => self.write_tdr(val),
            1 => self.programmed_reset(),
            2 => self.write_command(val),
            3 => self.control = val,
            _ => unreachable!("reg & 0x03 is always 0..=3"),
        }
    }

    /// IRQ output level: true while any IRQ source is armed.
    pub fn irq_asserted(&self) -> bool {
        self.status & status::IRQ != 0
    }

    /// Advance both frame timers and the DCD/DSR change-detector by `cycles`
    /// CPU cycles (crate clock, `CPU_HZ`).
    pub fn tick(&mut self, cycles: u32) {
        self.tick_modem_lines();
        self.tick_rx(cycles);
        self.tick_tx(cycles);
    }

    /// Take the next completed TX byte off the wire, if any (see
    /// `tx_output` field doc).
    pub fn take_tx_byte(&mut self) -> Option<u8> {
        self.tx_output.pop_front()
    }

    /// True when the receiver has no frame in progress and can accept a new
    /// byte using [`Self::receive_byte`].
    pub fn rx_ready(&self) -> bool {
        self.rx_timer.is_none()
    }

    /// Deliver one wire byte to the receiver; caller must check
    /// [`Self::rx_ready`] first. Lands in RDR after one frame's cycles.
    pub fn receive_byte(&mut self, b: u8) {
        debug_assert!(
            self.rx_ready(),
            "receive_byte called while a frame is already in progress"
        );
        self.rx_pending_byte = b;
        self.rx_timer = Some(self.cycles_per_frame());
    }

    /// Set the DCD (carrier detect) input level. Status bit tracks it live;
    /// IRQ arming on a *change* resolves on the next [`Self::tick`].
    pub fn set_dcd(&mut self, level: bool) {
        self.dcd_level = level;
        if level {
            self.status |= status::DCD;
        } else {
            self.status &= !status::DCD;
        }
    }

    /// Set the DSR input level; see [`Self::set_dcd`].
    pub fn set_dsr(&mut self, level: bool) {
        self.dsr_level = level;
        if level {
            self.status |= status::DSR;
        } else {
            self.status &= !status::DSR;
        }
    }

    /// Current RTS output — tracked but not wired to anything yet (see the
    /// `rts` field doc).
    pub fn rts(&self) -> bool {
        self.rts
    }
}

#[cfg(test)]
#[path = "acia6551_test.rs"]
mod tests;
