//! Tandy Sound/Speech Cartridge (SSC, 26-3144): the `$FF7D`/`$FF7E` bus
//! handshake, the cartridge's TMS7040 microcontroller running its real
//! firmware, the 2 KB static RAM, an AY-3-8913 PSG and an SP0256-AL2 speech
//! chip mixed into the machine's audio output, and the Sound Activity
//! Circuit (MAME `coco_ssc.cpp`).
//!
//! Everything the host sees is the firmware's doing: it interprets the
//! byte-stream protocol of the 26-3144 manual's Appendix A — command bytes
//! that load an 8×64-byte buffer area and execute it as sound data,
//! register strings, allophone streams or English text — over the four
//! ports the [`board`] module wires to the chips. The protocol's constants
//! live in [`commands`] for tests and tools; see wiki `cocovm/ssc-spec`.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tms7000::{StepKind, TMS7040};

use crate::ay8913::AY8913;
use crate::cart::{Cartridge, IO_OPEN_BUS};
use crate::sp0256::SP0256;

mod board;
mod commands;
mod sac;

pub use board::ALLOPHONE_COUNT;
pub use commands::{cmd, group, ram, terminator};

use board::{Board, BoardView};

/// Register addresses (MAME `coco_ssc.cpp`; SEB Unravelled II Appendix A;
/// wiki `cocovm/cartridges` "Carts can decode addresses outside SCS").
pub mod reg {
    /// SP0256 reset control (write) / always `0xFF` (read).
    pub const RESET: u16 = 0xFF7D;
    /// Host command latch (write) / status byte (read).
    pub const DATA: u16 = 0xFF7E;
}

/// `$FF7D` write: only bit 0 is decoded (the SP0256's RESET pin; its
/// falling edge also resets the TMS7040 and the AY).
const RESET_BIT: u8 = 0x01;

/// `$FF7E` status byte bit layout (MAME `coco_ssc_device::ff7e_r`).
mod status {
    /// Bits 4-0 read back set unconditionally — undocumented/unused status
    /// lines that MAME's real-hardware trace shows pulled high.
    pub const BASE: u8 = 0x1F;
    /// Bit 7: busy/ready. Set (1) when NOT busy; clear from a `$FF7E` write
    /// until the firmware raises port C bit 7.
    pub const NOT_BUSY: u8 = 0x80;
    /// Bit 6: SP0256 SBY ("standby" = idle/ready), straight from the chip.
    pub const SPEECH_READY: u8 = 0x40;
    /// Bit 5: Sound Activity Circuit output, *inverted* — 1 = quiet, 0 =
    /// sound is playing (MAME returns `!sound_active`).
    pub const QUIET: u8 = 0x20;
}

/// AY-3-8913 master clock = 2× the CoCo E-clock (MAME `coco_ssc.cpp`: the
/// PSG and the TMS7040 share one crystal at `DERIVED_CLOCK(2, 1)`). The
/// TMS7040 divides that by two internally, so it runs one cycle per E-cycle.
const AY_CLOCK_MULTIPLIER: u32 = 2;

/// Speech level relative to the PSG on the cartridge's output: MAME routes
/// the SP0256 at 1.75 and the AY-3-8913 at 2.0 (`coco_ssc.cpp`
/// `SP0256_GAIN`/`AY8913_GAIN`).
const SPEECH_GAIN: f32 = 1.75 / 2.0;

/// Widest cycle debt one TMS7040 step can leave (its longest instruction
/// is 49 cycles); a restored budget outside `[-MAX_STEP_DEBT, 0]` is corrupt.
const MAX_STEP_DEBT: i32 = 64;

/// A ROM image handed to [`SoundSpeechCartridge::new`] had the wrong size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SSCROMError {
    Firmware(tms7000::ROMSizeError),
    Speech(crate::sp0256::ROMSizeError),
}

impl std::fmt::Display for SSCROMError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SSCROMError::Firmware(e) => write!(f, "TMS7040 firmware: {e}"),
            SSCROMError::Speech(e) => write!(f, "SP0256-AL2 ROM: {e}"),
        }
    }
}

impl std::error::Error for SSCROMError {}

/// One firmware instruction about to execute, for trace tooling
/// ([`SoundSpeechCartridge::enable_firmware_trace`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirmwareTraceEntry {
    pub pc: u16,
    pub a: u8,
    pub b: u8,
    pub st: u8,
    pub sp: u8,
    /// The TMS7040's cycle counter before the instruction.
    pub cycles: u64,
}

/// The Sound/Speech Cartridge. Built around its two ROMs: the TMS7040
/// firmware and the SP0256-AL2 allophone ROM; neither is snapshotted, restore
/// re-supplies them through [`Self::reattach_firmware_rom`] and
/// [`Self::reattach_speech_rom`] like any other cart ROM.
#[derive(Serialize, Deserialize)]
pub struct SoundSpeechCartridge {
    ay: AY8913,
    /// Defaults to a reset chip when omitted from the snapshot payload.
    #[serde(default)]
    sp0256: SP0256,
    /// The microcontroller; an omitted field restores it with its reset
    /// pending, so the firmware boots on the first tick after reattachment.
    #[serde(default)]
    tms: TMS7040,
    #[serde(default)]
    board: Board,
    /// E-cycles owed to the TMS7040: [`Cartridge::tick`] runs instructions
    /// while positive and carries the overshoot (always `<= 0` between ticks).
    #[serde(default)]
    tms_budget: i32,
    /// Bit 0 of the last `$FF7D` write, for falling-edge detection. Primed
    /// high at power-on (MAME `m_reset_line = 1`), so the first write with
    /// bit 0 clear *is* an edge.
    prev_reset_bit0: bool,
    // Sound Activity Circuit state (see the [`sac`] module doc comment).
    sac_hpf_prev_in: f32,
    sac_hpf_prev_out: f32,
    sac_envelope: f32,
    /// True while the SAC considers the cartridge's own output "playing"
    /// (drives `$FF7E` bit 5, inverted).
    sac_sound_active: bool,
    /// Firmware instruction trace ring, when enabled (host-only tooling).
    #[serde(skip)]
    trace: Option<VecDeque<FirmwareTraceEntry>>,
    #[serde(skip)]
    trace_capacity: usize,
}

impl SoundSpeechCartridge {
    /// A cartridge around its 4 KB TMS7040 firmware and 2 KB SP0256-AL2 ROM,
    /// with the firmware's reset pending for the first tick.
    pub fn new(firmware: &[u8], sp0256_rom: &[u8]) -> Result<Self, SSCROMError> {
        Ok(Self {
            ay: AY8913::new(),
            sp0256: SP0256::new(sp0256_rom).map_err(SSCROMError::Speech)?,
            tms: TMS7040::new(firmware).map_err(SSCROMError::Firmware)?,
            board: Board::default(),
            tms_budget: 0,
            prev_reset_bit0: true,
            sac_hpf_prev_in: 0.0,
            sac_hpf_prev_out: 0.0,
            sac_envelope: 0.0,
            sac_sound_active: false,
            trace: None,
            trace_capacity: 0,
        })
    }

    /// Restore-path-only: re-supply the TMS7040's firmware after a snapshot
    /// restore; the chip keeps its state.
    pub fn reattach_firmware_rom(&mut self, rom: &[u8]) -> Result<(), tms7000::ROMSizeError> {
        self.tms.reattach_rom(rom)
    }

    /// Restore-path-only: re-supply the SP0256's ROM after a snapshot
    /// restore; the chip keeps its sequencer state.
    pub fn reattach_speech_rom(&mut self, rom: &[u8]) -> Result<(), crate::sp0256::ROMSizeError> {
        self.sp0256.reattach_rom(rom)
    }

    /// Direct AY-3-8913 register write, bypassing the firmware (tests/debugging).
    pub fn ay_write(&mut self, reg: u8, val: u8) {
        self.ay.write_reg(reg, val);
    }

    /// Direct AY-3-8913 register read (see [`SoundSpeechCartridge::ay_write`]).
    pub fn ay_read(&mut self, reg: u8) -> u8 {
        self.ay.read_reg(reg)
    }

    /// The last byte a `$FF7E` write latched into port A, for tests/debug.
    pub fn host_latch(&self) -> u8 {
        self.board.port_a
    }

    /// The host-visible BUSY* flag.
    pub fn busy(&self) -> bool {
        self.board.busy
    }

    /// The microcontroller, for tests and debugging.
    pub fn firmware(&self) -> &TMS7040 {
        &self.tms
    }

    /// The board's 2 KB static RAM, for tests and debugging.
    pub fn ram(&self) -> &[u8] {
        &self.board.ram
    }

    /// Keep the last `capacity` firmware instructions for [`Self::drain_firmware_trace`].
    pub fn enable_firmware_trace(&mut self, capacity: usize) {
        self.trace = Some(VecDeque::with_capacity(capacity));
        self.trace_capacity = capacity;
    }

    /// Take the firmware instructions recorded since the last drain.
    pub fn drain_firmware_trace(&mut self) -> Vec<FirmwareTraceEntry> {
        self.trace
            .as_mut()
            .map(|t| t.drain(..).collect())
            .unwrap_or_default()
    }

    fn trace_entry(&self) -> Option<FirmwareTraceEntry> {
        self.trace.as_ref()?;
        Some(FirmwareTraceEntry {
            pc: self.tms.pc,
            a: self.tms.a(),
            b: self.tms.b(),
            st: self.tms.st,
            sp: self.tms.sp,
            cycles: self.tms.cycles,
        })
    }

    fn record_trace(&mut self, entry: Option<FirmwareTraceEntry>) {
        if let (Some(entry), Some(trace)) = (entry, self.trace.as_mut()) {
            if trace.len() >= self.trace_capacity {
                trace.pop_front();
            }
            trace.push_back(entry);
        }
    }

    /// Run the firmware for the E-cycles owed so far, one instruction (or
    /// interrupt entry, or reset) at a time, carrying any overshoot.
    fn run_firmware(&mut self) {
        while self.tms_budget > 0 {
            let entry = self.trace_entry();
            let step = {
                let mut view = BoardView {
                    board: &mut self.board,
                    ay: &mut self.ay,
                    sp0256: &mut self.sp0256,
                };
                self.tms.step(&mut view)
            };
            self.tms_budget -= step.cycles as i32;
            if step.kind == StepKind::Instruction {
                self.record_trace(entry);
            }
            self.sync_interrupt_lines();
        }
    }

    /// INT3 follows the host-byte latch (dropped by the firmware's port A
    /// read); INT1 is the SP0256's load request, which MAME wires as DRQ
    /// and which equals LRQ.
    fn sync_interrupt_lines(&mut self) {
        self.tms.set_int3(self.board.int3);
        self.tms.set_int1(self.sp0256.lrq());
    }

    /// `$FF7D` write: bit 0 is the SP0256's RESET pin (every write with it
    /// set resets the chip); its falling edge also resets the TMS7040 and
    /// the AY and clears BUSY* (MAME `ff7d_write`).
    fn write_reset(&mut self, val: u8) {
        let bit0 = val & RESET_BIT != 0;
        let falling_edge = self.prev_reset_bit0 && !bit0;
        self.prev_reset_bit0 = bit0;
        if bit0 {
            self.sp0256.reset();
        }
        if falling_edge {
            self.tms.assert_reset();
            self.ay.reset();
            self.board.busy = false;
        }
    }

    /// `$FF7E` write: latch the byte into port A, raise BUSY* and INT3. A
    /// byte the firmware hasn't read yet is simply overwritten — the manual's
    /// "you lose data" while busy (MAME `ff7d_write` case 1).
    fn write_data(&mut self, val: u8) {
        self.board.port_a = val;
        self.board.busy = true;
        self.board.int3 = true;
        self.tms.set_int3(true);
    }

    fn read_data(&self) -> u8 {
        let mut s = status::BASE;
        if !self.board.busy {
            s |= status::NOT_BUSY;
        }
        if self.sp0256.sby() {
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

    /// The firmware runs first so the strobes it issues within these cycles
    /// land before the chips step over them; then the PSG and speech chip
    /// advance, and INT1 picks up a load request the SP0256 raised meanwhile.
    fn tick(&mut self, cycles: u32) {
        self.tms_budget += cycles as i32;
        self.run_firmware();
        self.ay.step(cycles * AY_CLOCK_MULTIPLIER);
        self.sp0256.step(cycles);
        self.tms.set_int1(self.sp0256.lrq());
    }

    /// Machine reset: every chip resets; the static RAM keeps its contents.
    fn reset(&mut self) {
        self.ay.reset();
        self.sp0256.reset();
        self.tms.assert_reset();
        self.board.busy = false;
        self.board.int3 = false;
        self.tms.set_int3(false);
        self.tms_budget = 0;
        self.prev_reset_bit0 = true;
        self.sac_hpf_prev_in = 0.0;
        self.sac_hpf_prev_out = 0.0;
        self.sac_envelope = 0.0;
        self.sac_sound_active = false;
    }

    /// Drains the AY's output and feeds the Sound Activity Circuit
    /// unconditionally — `$FF7E` bit 5 must reflect the cartridge's own
    /// output regardless of whether the sound mux is selecting it — then
    /// adds the speech chip, which bypasses the SAC as on MAME's board.
    fn audio_sample(&mut self) -> f32 {
        let psg = self.ay.drain();
        self.update_sac(psg);
        psg + SPEECH_GAIN * self.sp0256.output()
    }

    /// Rebuilds `ay.dac` — pure construction-time scratch, skipped from the
    /// snapshot (`Ay8913::after_restore`).
    fn after_restore(&mut self) {
        self.ay.after_restore();
    }

    /// Restore-only: a hand-edited cycle budget outside what one step can
    /// leave would spin the firmware loop or stall it; a hand-edited TMS7040
    /// (its timer1 in particular) can panic or spin the same way.
    fn validate_restored(&self) -> Result<(), String> {
        if self.tms_budget > 0 || self.tms_budget < -MAX_STEP_DEBT {
            return Err(format!(
                "SSC: tms_budget ({}) outside [-{MAX_STEP_DEBT}, 0]",
                self.tms_budget
            ));
        }
        self.tms
            .validate()
            .map_err(|e| format!("SSC: TMS7040 {e}"))?;
        Ok(())
    }
}
