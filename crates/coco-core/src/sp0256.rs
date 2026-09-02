//! GI SP0256-AL2 "Narrator" speech processor: a microsequencer walking the
//! 2 KB allophone mask ROM to drive a 12-pole LPC filter, as fitted to the
//! Tandy Sound/Speech Cartridge. Port of MAME `src/devices/sound/sp0256.cpp`
//! (Joe Zbiciak's core, ported to MAME by Tim Lindner); every claim here is
//! cited against that file. The SPB640 speech FIFO is not modelled — nothing
//! on the SSC drives it.
//!
//! Clocking: the chip has its own 3.12 MHz crystal and emits one sample per
//! 312 clocks (10 kHz). [`SP0256::step`] converts E-clock cycles to that
//! rate with a fixed ratio, so status lines advance deterministically even
//! when no audio device is draining output. [`SP0256::output`] linearly
//! interpolates between the last two samples — a cheap stand-in for the
//! board's RC low-pass on the chip's digital output.

use std::fmt;

use serde::{Deserialize, Serialize};

mod datafmt;
mod lpc;
mod micro;

use lpc::Lpc12;

/// The AL2 mask ROM's size.
pub const ROM_SIZE: usize = 2048;
/// Where the AL2 ROM sits in the chip's 64 KB address space (MAME
/// `coco_ssc.cpp` `ROM_LOAD("sp0256-al2.bin", 0x1000, ...)`).
const ROM_BASE: u32 = 0x1000;
/// [`ROM_BASE`] as a bit address: the sequencer's reset page, and the base
/// every ALD command address is OR'd onto (MAME `m_page = 0x1000 << 3`).
const ROM_BASE_BITS: u32 = ROM_BASE << 3;
/// Address bus width mask, in bytes.
const ADDRESS_MASK: u32 = 0xFFFF;
/// An ALD value `n` selects the 2-byte jump-table slot at ROM byte `2n`,
/// i.e. bit address `n << 4` (MAME `m_ald = data << 4`).
const ALD_SHIFT: u32 = 4;
/// The SSC's crystal (MAME `coco_ssc.cpp` `SP0256(config, m_spo, XTAL(3'120'000))`).
const CRYSTAL_HZ: u64 = 3_120_000;
/// Crystal clocks per output sample (MAME `CLOCK_DIVIDER (6*4*13)`).
const CLOCK_DIVIDER: u64 = 6 * 4 * 13;
/// Output sample rate: 10 kHz.
pub const SAMPLE_RATE_HZ: u64 = CRYSTAL_HZ / CLOCK_DIVIDER;
/// E-clock rate the fixed cycle-to-sample ratio is taken against.
const E_CLOCK_HZ: u64 = crate::CPU_HZ as u64;
/// Full-scale divisor for the 16-bit filter output.
const OUTPUT_SCALE: f32 = 32768.0;

/// The ROM image handed to [`SP0256::new`] wasn't exactly [`ROM_SIZE`] bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ROMSizeError {
    pub actual: usize,
}

impl fmt::Display for ROMSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SP0256-AL2 ROM must be exactly {ROM_SIZE} bytes, got {}",
            self.actual
        )
    }
}

impl std::error::Error for ROMSizeError {}

/// Sequencer + filter state. The ROM is `#[serde(skip)]`: snapshots carry
/// the chip's state, and the frontend reattaches the image on restore
/// ([`SP0256::reattach_rom`]).
#[derive(Serialize, Deserialize)]
pub struct SP0256 {
    #[serde(skip)]
    rom: Box<[u8]>,
    /// SBY (standby) line: high while idle.
    sby: bool,
    /// LRQ (load request) line: high when the ALD latch can take a command.
    lrq: bool,
    /// Latched ALD command, as a bit address offset (see [`ALD_SHIFT`]).
    ald: u32,
    /// Program counter, in bits.
    pc: u32,
    /// One-deep return-address stack (0 = empty, so RTS halts).
    stack: u32,
    halted: bool,
    /// Mode register — see `micro.rs`.
    mode: u8,
    /// Current page for JMP/JSR targets, in bits.
    page: u32,
    /// Set by PAUSE/clear-all until a register load lands.
    silent: bool,
    filt: Lpc12,
    /// Fractional-sample accumulator in units of `E-cycles × SAMPLE_RATE_HZ`.
    cycle_acc: u64,
    prev_sample: i16,
    cur_sample: i16,
}

impl SP0256 {
    /// Build a chip around a 2 KB AL2 ROM image, in reset.
    pub fn new(rom: &[u8]) -> Result<Self, ROMSizeError> {
        let mut chip = Self {
            rom: Box::default(),
            sby: true,
            lrq: true,
            ald: 0,
            pc: 0,
            stack: 0,
            halted: true,
            mode: 0,
            page: ROM_BASE_BITS,
            silent: true,
            filt: Lpc12::default(),
            cycle_acc: 0,
            prev_sample: 0,
            cur_sample: 0,
        };
        chip.reattach_rom(rom)?;
        Ok(chip)
    }

    /// Restore-path: re-inject the ROM image a snapshot doesn't carry.
    pub fn reattach_rom(&mut self, rom: &[u8]) -> Result<(), ROMSizeError> {
        if rom.len() != ROM_SIZE {
            return Err(ROMSizeError { actual: rom.len() });
        }
        self.rom = rom.into();
        Ok(())
    }

    /// The RESET pin (MAME `device_reset`): halt, release LRQ and SBY, and
    /// zero the filter. Output resamplers are left alone.
    pub fn reset(&mut self) {
        self.filt = Lpc12::default();
        self.halted = true;
        self.lrq = true;
        self.ald = 0;
        self.pc = 0;
        self.stack = 0;
        self.mode = 0;
        self.page = ROM_BASE_BITS;
        self.silent = true;
        self.sby = true;
    }

    /// Address LoaD: latch a command (allophone) address. Dropped while LRQ
    /// is low — the previous command hasn't been picked up yet (MAME `ald_w`).
    pub fn ald_write(&mut self, address: u8) {
        if !self.lrq {
            return;
        }
        self.lrq = false;
        self.ald = u32::from(address) << ALD_SHIFT;
        self.sby = false;
    }

    /// LRQ: true when [`SP0256::ald_write`] would be accepted.
    pub fn lrq(&self) -> bool {
        self.lrq
    }

    /// SBY: true while the sequencer is halted with nothing pending.
    pub fn sby(&self) -> bool {
        self.sby
    }

    /// Advance by `e_cycles` E-clock cycles, generating whole 10 kHz samples
    /// as the fixed ratio accrues them.
    pub fn step(&mut self, e_cycles: u32) {
        self.cycle_acc += u64::from(e_cycles) * SAMPLE_RATE_HZ;
        while self.cycle_acc >= E_CLOCK_HZ {
            self.cycle_acc -= E_CLOCK_HZ;
            self.prev_sample = self.cur_sample;
            self.cur_sample = self.generate_sample();
        }
    }

    /// Current output in `[-1.0, 1.0]`, interpolated between the last two
    /// samples by the fraction of a sample period elapsed since the newest.
    pub fn output(&self) -> f32 {
        let phase = self.cycle_acc as f32 / E_CLOCK_HZ as f32;
        let prev = f32::from(self.prev_sample) / OUTPUT_SCALE;
        let cur = f32::from(self.cur_sample) / OUTPUT_SCALE;
        prev + (cur - prev) * phase
    }

    /// Produce the next raw 10 kHz sample, running the sequencer whenever the
    /// filter's repeat count expires (MAME `sound_stream_update`'s inner loop).
    pub fn generate_sample(&mut self) -> i16 {
        loop {
            if self.filt.rpt <= 0 {
                self.micro();
            }
            if let Some(sample) = self.filt.update_one() {
                return sample;
            }
        }
    }

    /// ROM read at a byte address; anything outside the 2 KB image reads
    /// as 0 (an RTS/HLT instruction), matching MAME's zero-filled region.
    fn rom_byte(&self, byte_addr: u32) -> u8 {
        (byte_addr & ADDRESS_MASK)
            .checked_sub(ROM_BASE)
            .and_then(|off| self.rom.get(off as usize))
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[path = "sp0256_test.rs"]
mod tests;
