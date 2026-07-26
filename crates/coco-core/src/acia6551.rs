//! 6551 ACIA (MOS 6551 / Rockwell R6551 / WDC W65C51 — not a Motorola
//! MC-prefixed part), the UART at the heart of the Tandy Deluxe RS-232
//! Program Pak. Register/bit semantics are MAME-authoritative
//! (`src/devices/machine/mos6551.cpp`, master) per `docs/plan-deluxe-rs232.md`
//! task 2 — every fact below was checked against that source, not derived
//! from a datasheet.
//!
//! # Byte-level timing divergence
//!
//! MAME's `mos6551_device` is a bit-serial engine: it shifts one bit at a
//! time off a per-bit timer and can therefore generate real parity/framing
//! errors and expose bit-accurate RS-232 waveforms. This model is
//! deliberately **byte-level**: [`Acia6551::tick`] runs a whole-frame timer
//! for the receiver and transmitter, sized from the same baud-rate math MAME
//! uses (see [`BAUD_DIVIDER`]), and delivers/consumes a complete byte when
//! that timer expires. Consequences of the divergence, called out again at
//! each relevant spot below:
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
//!   [`Acia6551::stop_bits`]).
//! - DCD/DSR level-change IRQ arming is checked once per [`Acia6551::tick`]
//!   call rather than on a live edge — MAME itself ties this to the receive
//!   clock and carries `TODO` comments admitting the exact timing is
//!   unresolved, so tying it to our own tick boundary is no worse and is
//!   simpler to reason about. See [`Acia6551::tick_modem_lines`].
//!
//! # Wire interface
//!
//! This module is a pure chip model with a byte-level wire interface: no
//! knowledge of hosts, sockets, or files. [`Acia6551::take_tx_byte`] /
//! [`Acia6551::receive_byte`] / [`Acia6551::rx_ready`] / [`Acia6551::set_dcd`]
//! / [`Acia6551::set_dsr`] are the seam a future cartridge-glue layer drives
//! from a real host endpoint.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

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

/// Status register bits (offset 1, read-only view; see [`Acia6551::read`]).
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
    /// idles the transmitter — a written TDR just sits, TDRE stays clear,
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
    /// [`Acia6551::frame_bits`]) — this model does not distinguish which
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
/// [`Acia6551::cycles_per_frame`].
pub mod control {
    /// Bits 3:0: baud-rate index — see [`BAUD_DIVIDER`].
    pub const BAUD_MASK: u8 = 0x0F;
    /// Bit 4: receiver clock source. Stored for completeness; this model
    /// has no external-clock behavior to switch (see [`BAUD_DIVIDER`]'s
    /// index-0 note).
    pub const RX_CLOCK_SOURCE: u8 = 0x10;
    /// Bits 6:5: word length field — mask before shifting by
    /// [`WORD_LENGTH_SHIFT`]; `word_length = 8 - field`.
    pub const WORD_LENGTH_MASK: u8 = 0x60;
    /// Shift to bring [`WORD_LENGTH_MASK`] down to a 0..=3 value.
    pub const WORD_LENGTH_SHIFT: u8 = 5;
    /// Bit 7: 2 stop bits instead of 1 (also covers the 5-bit-word 1.5-stop
    /// case, collapsed to 2 — see [`Acia6551::stop_bits`]).
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
pub struct Acia6551 {
    /// Receive Data Register — last completed RX byte.
    rdr: u8,
    /// Transmit Data Register — last byte written by the CPU, pending
    /// consumption by the transmitter.
    tdr: u8,
    /// Command register (offset 2).
    command: u8,
    /// Control register (offset 3).
    control: u8,
    /// Status register (offset 1) as it currently stands, including the
    /// live IRQ bit — kept in sync by [`Self::update_irq_output`] whenever
    /// `irq_sources` changes.
    status: u8,
    /// Bitmask of currently-armed IRQ sources (see [`irq_source`]).
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
    /// yet (`docs/plan-deluxe-rs232.md` task 2 spec).
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

impl Default for Acia6551 {
    fn default() -> Self {
        Self::new()
    }
}

impl Acia6551 {
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

    /// RESET* pin: status becomes TDRE-only (plus DCD/DSR bits reflecting
    /// whatever the input levels currently are), command and control both
    /// go to 0, every IRQ source is cleared, and both frame timers are
    /// idled (MAME `mos6551_device::device_reset`).
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
        // Re-baseline the change-detectors so the next tick doesn't treat
        // the reset itself as a DCD/DSR edge.
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
    /// CPU cycles (crate clock, [`CPU_HZ`]).
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
    /// byte via [`Self::receive_byte`].
    pub fn rx_ready(&self) -> bool {
        self.rx_timer.is_none()
    }

    /// Deliver one wire byte to the receiver. The caller must have checked
    /// [`Self::rx_ready`] first; the byte lands in RDR after one frame's
    /// worth of cycles (see [`Self::cycles_per_frame`]), modeling the wire
    /// time of the serial frame.
    pub fn receive_byte(&mut self, b: u8) {
        debug_assert!(
            self.rx_ready(),
            "receive_byte called while a frame is already in progress"
        );
        self.rx_pending_byte = b;
        self.rx_timer = Some(self.cycles_per_frame());
    }

    /// Set the DCD (carrier detect) input level. The status bit tracks the
    /// live level immediately and unconditionally; the IRQ-source arming on
    /// a *change* is resolved on the next [`Self::tick`] (module doc,
    /// "byte-level timing divergence").
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

    // ---- register read/write internals ----------------------------------

    /// RDR read: returns the byte, then clears RDRF and all three error
    /// bits together (MAME `mos6551.cpp` `read_receive_data_register` — does
    /// *not* touch the IRQ output; that needs a status read or disabling
    /// the RDRF IRQ source via a command write).
    fn read_rdr(&mut self) -> u8 {
        let val = self.rdr;
        self.status &=
            !(status::RDRF | status::PARITY_ERROR | status::FRAMING_ERROR | status::OVERRUN);
        val
    }

    /// Status read: returns the pre-clear snapshot, then (side effect)
    /// clears every armed IRQ source at once and drops the IRQ output bit.
    /// Does not touch parity/framing/overrun/RDRF/DCD/DSR (MAME
    /// `mos6551.cpp` `read_status_register`).
    fn read_status(&mut self) -> u8 {
        let val = self.status;
        self.irq_sources = 0;
        self.update_irq_output();
        val
    }

    /// TDR write: latches the byte and clears TDRE immediately, even if a
    /// frame is already shifting (the 1-deep holding register — a rewrite
    /// before the current frame ends is picked up when it does, in
    /// [`Self::tick_tx`]). Then attempts an immediate consume — if the
    /// transmitter happens to already be idle, MAME picks a freshly loaded
    /// TDR up as soon as it's written rather than waiting for the next
    /// clock tick.
    fn write_tdr(&mut self, val: u8) {
        self.tdr = val;
        self.status &= !status::TDRE;
        self.start_tx_frame_if_ready();
    }

    /// Command register write (MAME `mos6551.cpp` `write_command`):
    /// replaces the whole register, then re-evaluates the two IRQ sources
    /// whose arming condition is a direct function of the command bits —
    /// disabling rx-IRQ clears the RDRF source, disabling tx-IRQ clears the
    /// TDRE source — and attempts an immediate transmit-start (DTR/BREAK
    /// may have just changed).
    fn write_command(&mut self, val: u8) {
        self.command = val;
        self.rts = self.tx_control() != tx_control::RTS_OFF;
        if !self.rx_irq_enabled() {
            self.irq_sources &= !irq_source::RDRF;
        }
        if !self.tx_irq_enabled() {
            self.irq_sources &= !irq_source::TDRE;
        }
        self.update_irq_output();
        self.start_tx_frame_if_ready();
    }

    /// Programmed reset (any write to reg 1, value ignored): clears *only*
    /// the overrun status bit and *only* the DCD/DSR IRQ-source bits — the
    /// RDRF/TDRE IRQ sources deliberately survive, unlike the general
    /// command-write rule in [`Self::write_command`] (verified MAME
    /// `mos6551.cpp` `write_status_command_register` behavior: this is a
    /// narrower reset than a full command-register disable would produce).
    /// Command bits 0-4 are cleared (DTR off, rx-IRQ enabled, transmitter
    /// control = RTS_OFF, echo off); parity bits 7:5 survive. Control is
    /// untouched.
    fn programmed_reset(&mut self) {
        self.status &= !status::OVERRUN;
        self.irq_sources &= !(irq_source::DCD | irq_source::DSR);
        self.update_irq_output();

        const RESET_MASK: u8 = command::DTR
            | command::RX_IRQ_DISABLE
            | command::TX_CONTROL_MASK
            | command::ECHO;
        self.command &= !RESET_MASK;
        self.rts = false;
    }

    // ---- IRQ bookkeeping --------------------------------------------------

    /// Recompute the [`status::IRQ`] bit from `irq_sources`.
    fn update_irq_output(&mut self) {
        if self.irq_sources != 0 {
            self.status |= status::IRQ;
        } else {
            self.status &= !status::IRQ;
        }
    }

    fn dtr_enabled(&self) -> bool {
        self.command & command::DTR != 0
    }

    /// Receiver IRQ armed: command bit 1 clear (enabled) AND DTR enabled
    /// (module doc: DTR disabled gates off rx-IRQ).
    fn rx_irq_enabled(&self) -> bool {
        self.dtr_enabled() && self.command & command::RX_IRQ_DISABLE == 0
    }

    /// Transmitter IRQ armed: transmitter control = `IRQ_ENABLED` AND DTR
    /// enabled.
    fn tx_irq_enabled(&self) -> bool {
        self.dtr_enabled() && self.tx_control() == tx_control::IRQ_ENABLED
    }

    fn tx_control(&self) -> u8 {
        (self.command & command::TX_CONTROL_MASK) >> command::TX_CONTROL_SHIFT
    }

    fn break_active(&self) -> bool {
        self.tx_control() == tx_control::BREAK
    }

    /// Check the DCD/DSR inputs for a change since the last tick and arm
    /// their IRQ source if DTR is enabled (module doc: MAME ties this to
    /// the RX clock with admittedly-unresolved exact timing; this model
    /// resolves it once per `tick` call instead).
    fn tick_modem_lines(&mut self) {
        if self.dcd_level != self.dcd_checked {
            self.dcd_checked = self.dcd_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DCD;
                self.update_irq_output();
            }
        }
        if self.dsr_level != self.dsr_checked {
            self.dsr_checked = self.dsr_level;
            if self.dtr_enabled() {
                self.irq_sources |= irq_source::DSR;
                self.update_irq_output();
            }
        }
    }

    // ---- TX/RX frame engine ----------------------------------------------

    /// If the transmitter is idle and a byte is pending (TDRE clear) and
    /// DTR is enabled and BREAK is not active: consume TDR into the shifter,
    /// set TDRE (freeing TDR for a new write) and arm the TDRE IRQ source if
    /// enabled, and start the frame timer. This is the "consume-at-start"
    /// moment MAME fires TDRE/IRQ at — deliberately *not* at frame end (see
    /// [`Self::complete_tx_frame`]).
    fn start_tx_frame_if_ready(&mut self) {
        if self.tx_timer.is_some() {
            return;
        }
        if !self.dtr_enabled() || self.break_active() {
            return;
        }
        if self.status & status::TDRE != 0 {
            return; // no pending byte
        }
        self.tx_shift_byte = self.tdr;
        self.status |= status::TDRE;
        if self.tx_irq_enabled() {
            self.irq_sources |= irq_source::TDRE;
            self.update_irq_output();
        }
        self.tx_timer = Some(self.cycles_per_frame());
    }

    /// Frame-end: the shifted byte becomes available on the wire and the
    /// transmitter is free again (the next consume, if TDR was rewritten
    /// mid-frame, happens on the following [`Self::tick_tx`] iteration).
    fn complete_tx_frame(&mut self) {
        self.tx_output.push_back(self.tx_shift_byte);
    }

    fn tick_rx(&mut self, cycles: u32) {
        if let Some(remaining) = self.rx_timer {
            if cycles >= remaining {
                self.rx_timer = None;
                self.complete_rx_frame();
            } else {
                self.rx_timer = Some(remaining - cycles);
            }
        }
    }

    /// Runs the TX frame timer to completion, possibly chaining straight
    /// into the next frame within the same `tick` call if a byte was
    /// already pending and `cycles` outlasts the current frame.
    fn tick_tx(&mut self, mut cycles: u32) {
        loop {
            if self.tx_timer.is_none() {
                self.start_tx_frame_if_ready();
            }
            let Some(remaining) = self.tx_timer else {
                break;
            };
            if cycles >= remaining {
                cycles -= remaining;
                self.tx_timer = None;
                self.complete_tx_frame();
            } else {
                self.tx_timer = Some(remaining - cycles);
                break;
            }
        }
    }

    /// RX frame completion: the pending byte always replaces RDR. Overrun
    /// is set if RDRF was already set (the previous byte was never read).
    /// RDRF is set unconditionally; its IRQ source is armed only if rx-IRQ
    /// is enabled. In echo mode the byte is also queued onto the TX wire
    /// (module doc's byte-level echo approximation — this skips MAME's
    /// force-to-mark-during-overrun nuance).
    fn complete_rx_frame(&mut self) {
        let byte = self.rx_pending_byte;
        if self.status & status::RDRF != 0 {
            self.status |= status::OVERRUN;
        }
        self.rdr = byte;
        self.status |= status::RDRF;
        if self.rx_irq_enabled() {
            self.irq_sources |= irq_source::RDRF;
            self.update_irq_output();
        }
        if self.command & command::ECHO != 0 {
            self.tx_output.push_back(byte);
        }
    }

    // ---- baud/frame timing -------------------------------------------------

    fn baud_divider(&self) -> u32 {
        BAUD_DIVIDER[(self.control & control::BAUD_MASK) as usize]
    }

    /// `8 - field`, field = control bits 6:5 (MAME `mos6551.cpp`
    /// `write_control`: 00=8, 01=7, 10=6, 11=5 data bits).
    fn word_length(&self) -> u32 {
        8 - u32::from((self.control & control::WORD_LENGTH_MASK) >> control::WORD_LENGTH_SHIFT)
    }

    /// Any odd parity-field value (odd/even/mark/space) adds one bit to the
    /// frame; even values mean parity disabled (see [`command::PARITY_MASK`]
    /// doc).
    fn parity_enabled(&self) -> bool {
        let field = (self.command & command::PARITY_MASK) >> command::PARITY_SHIFT;
        field & 1 != 0
    }

    /// 1, or 2 with [`control::STOP_BITS_2`] set. MAME's real 6551 gives a
    /// 5-bit word + 2-stop-bits combination 1.5 stop bits instead of 2; this
    /// byte-level model collapses that case to a plain 2 (module doc).
    fn stop_bits(&self) -> u32 {
        if self.control & control::STOP_BITS_2 != 0 {
            2
        } else {
            1
        }
    }

    fn frame_bits(&self) -> u32 {
        START_BITS + self.word_length() + u32::from(self.parity_enabled()) + self.stop_bits()
    }

    /// CPU cycles (crate clock, [`CPU_HZ`]) for one complete RX or TX frame
    /// at the currently configured baud/word-length/parity/stop-bits:
    /// `cycles_per_frame = frame_bits * divider * `[`BAUD_CLOCK_DIVISOR`]`
    /// `* `[`CPU_HZ`]` / `[`ACIA_CRYSTAL_HZ`]`` (u64 math to avoid overflow
    /// and rounding surprises before the final division).
    fn cycles_per_frame(&self) -> u32 {
        let frame_bits = u64::from(self.frame_bits());
        let divider = u64::from(self.baud_divider());
        let cycles = frame_bits * divider * BAUD_CLOCK_DIVISOR * CPU_HZ / ACIA_CRYSTAL_HZ;
        cycles as u32
    }
}

#[cfg(test)]
#[path = "acia6551_test.rs"]
mod tests;
