//! The SP0256 microsequencer: fetches bit-packed instructions from the
//! allophone ROM (LSB-first, at a bit-granular PC), runs control transfers,
//! and unpacks operand blocks into the filter registers (MAME `sp0256.cpp`
//! `micro`/`getb`).

use super::datafmt::{DATAFMT, block_range};
use super::lpc::{PER_PAUSE, REGISTER_COUNT, reg};
use super::{ROM_BASE_BITS, SP0256};

/// Numeric opcodes as fetched (MAME's comments name them bit-reversed).
mod opcode {
    /// `immed4 == 0`: RTS (or HLT with an empty stack); otherwise SETPAGE.
    pub const RTS_SETPAGE: u8 = 0x0;
    /// Set mode bits and the repeat-count MSBs for the next instruction.
    pub const SETMODE: u8 = 0x1;
    pub const JSR: u8 = 0xD;
    pub const JMP: u8 = 0xE;
    pub const PAUSE: u8 = 0xF;
}

/// Mode register: bits 1-2 select the operand layout, bits 4-5 carry
/// SETMODE's repeat-count MSBs (consumed by the next instruction).
const MODE_LAYOUT_MASK: u8 = 0x0F;
const MODE_REPEAT_MSBS: u8 = 0x30;

/// Defensive cap on instructions fetched per sequencer run. Not a hardware
/// fact: the real chip runs until an operand block or HLT; this only keeps
/// a garbage ROM whose code loops on itself from hanging the emulator.
const STEP_BUDGET: u32 = 4096;

impl SP0256 {
    /// Fetch `len` (at most 8) bits at the bit-address PC, LSB-first across
    /// the byte boundary, then advance the PC.
    fn getb(&mut self, len: u32) -> u32 {
        let d0 = u32::from(self.rom_byte(self.pc >> 3));
        let d1 = u32::from(self.rom_byte((self.pc + 8) >> 3));
        let data = ((d1 << 8) | d0) >> (self.pc & 7);
        self.pc += len;
        data & ((1 << len) - 1)
    }

    /// Run instructions until an operand block gives the filter a repeat
    /// count, or the sequencer halts. Kept as one function, over the usual
    /// length ceiling, so it audits line by line against MAME's `micro`.
    pub(super) fn micro(&mut self) {
        let mut budget = STEP_BUDGET;
        while self.filt.rpt <= 0 {
            if self.halted && !self.lrq {
                self.pc = self.ald | ROM_BASE_BITS;
                self.halted = false;
                self.lrq = true;
                self.ald = 0;
                self.filt.r = [0; REGISTER_COUNT];
            }
            if self.halted {
                self.filt.rpt = 1;
                self.lrq = true;
                self.ald = 0;
                self.filt.r = [0; REGISTER_COUNT];
                self.sby = true;
                return;
            }
            if budget == 0 {
                self.halted = true;
                continue;
            }
            budget -= 1;

            let immed4 = self.getb(4) as u8;
            let opcode = self.getb(4) as u8;
            let mut repeat = 0u8;
            let mut ctrl_xfer = false;
            match opcode {
                opcode::RTS_SETPAGE => {
                    if immed4 != 0 {
                        self.page = u32::from(immed4).reverse_bits() >> 13;
                    } else {
                        let target = self.stack;
                        self.stack = 0;
                        if target == 0 {
                            self.halted = true;
                            self.pc = 0;
                        } else {
                            self.pc = target;
                        }
                        ctrl_xfer = true;
                    }
                }
                opcode::JMP | opcode::JSR => {
                    let target = self.page
                        | (u32::from(immed4).reverse_bits() >> 17)
                        | (self.getb(8).reverse_bits() >> 21);
                    ctrl_xfer = true;
                    if opcode == opcode::JSR {
                        // Return address is byte-aligned.
                        self.stack = (self.pc + 7) & !7;
                    }
                    self.pc = target;
                }
                opcode::SETMODE => {
                    self.mode = ((immed4 & 8) >> 2) | (immed4 & 4) | ((immed4 & 3) << 4);
                }
                _ => repeat = immed4 | (self.mode & MODE_REPEAT_MSBS),
            }
            if opcode != opcode::SETMODE {
                self.mode &= MODE_LAYOUT_MASK;
            }
            if ctrl_xfer || repeat == 0 {
                continue;
            }

            // "repeat + 1" compensates for regdec forcing an immediate impulse.
            self.filt.rpt = i32::from(repeat) + 1;
            let range = block_range(opcode, self.mode)
                .expect("every opcode reaching an operand block has a layout row");
            self.apply_operand_block(range);
            if opcode == opcode::PAUSE {
                self.silent = true;
                self.filt.r[reg::PERIOD] = PER_PAUSE;
            }
            self.filt.regdec();
            break;
        }
    }

    /// Unpack one operand block (an inclusive [`DATAFMT`] row range) into
    /// the filter registers.
    fn apply_operand_block(&mut self, (first, last): (usize, usize)) {
        for field in &DATAFMT[first..=last] {
            if field.clear_all {
                self.filt.r = [0; REGISTER_COUNT];
                self.silent = true;
            }
            if field.clear5 {
                self.filt.r[reg::B5] = 0;
                self.filt.r[reg::F5] = 0;
            }
            if field.len == 0 {
                continue;
            }
            let len = u32::from(field.len);
            let raw = self.getb(len) as u8;
            let sign_bit = 1u8 << (field.len - 1);
            let mut value = raw as i8;
            if field.delta && raw & sign_bit != 0 {
                // MAME's `value |= -1 << len`; an 8-bit delta would need no
                // extension at all, and `wrapping_shl` would wrongly wrap it.
                debug_assert!(field.len < 8, "delta fields are at most 5 bits wide");
                value |= (-1i8).wrapping_shl(len);
            }
            if field.shift != 0 {
                value = value.wrapping_shl(u32::from(field.shift));
            }
            self.silent = false;
            let r = &mut self.filt.r[field.param];
            if field.field {
                let keep_low = ((1u16 << field.shift) - 1) as u8;
                *r = (*r & keep_low) | value as u8;
            } else if field.delta {
                *r = r.wrapping_add(value as u8);
            } else {
                *r = value as u8;
            }
        }
    }
}
