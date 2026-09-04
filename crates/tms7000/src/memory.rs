//! The chip's own address decode (MAME `tms7040_mem`) and the byte/word,
//! immediate, and stack accessors the opcodes are written against (MAME
//! `tms7000.h` inline helpers).

use crate::{Bus, PERIPHERAL_FILE_BASE, ROM_BASE, TMS7040};

/// Top of the register file; `$0080-$00FF` is unmapped on the 70x0 family
/// (reads 0, writes ignored — MAME `tms7000_unmapped_rf_r/w`).
const REGISTER_FILE_END: u16 = 0x007F;
const UNMAPPED_RF_END: u16 = 0x00FF;
/// Last peripheral-file register on the 70x0 family.
const PERIPHERAL_FILE_END: u16 = 0x010B;

impl TMS7040 {
    pub(crate) fn rom_byte(&self, addr: u16) -> u8 {
        self.rom
            .get(usize::from(addr - ROM_BASE))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn read_mem(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        match addr {
            0..=REGISTER_FILE_END => self.rf[usize::from(addr)],
            0x0080..=UNMAPPED_RF_END => 0,
            PERIPHERAL_FILE_BASE..=PERIPHERAL_FILE_END => {
                self.pf_read(bus, (addr - PERIPHERAL_FILE_BASE) as u8)
            }
            ROM_BASE..=0xFFFF => self.rom_byte(addr),
            _ => bus.read_ext(addr),
        }
    }

    pub(crate) fn write_mem(&mut self, bus: &mut impl Bus, addr: u16, val: u8) {
        match addr {
            0..=REGISTER_FILE_END => self.rf[usize::from(addr)] = val,
            0x0080..=UNMAPPED_RF_END => {}
            PERIPHERAL_FILE_BASE..=PERIPHERAL_FILE_END => {
                self.pf_write(bus, (addr - PERIPHERAL_FILE_BASE) as u8, val)
            }
            ROM_BASE..=0xFFFF => {}
            _ => bus.write_ext(addr, val),
        }
    }

    /// Big-endian word at `addr`, `addr + 1` (wrapping).
    pub(crate) fn read_mem16(&mut self, bus: &mut impl Bus, addr: u16) -> u16 {
        let hi = self.read_mem(bus, addr);
        let lo = self.read_mem(bus, addr.wrapping_add(1));
        u16::from_be_bytes([hi, lo])
    }

    pub(crate) fn write_mem16(&mut self, bus: &mut impl Bus, addr: u16, val: u16) {
        let [hi, lo] = val.to_be_bytes();
        self.write_mem(bus, addr, hi);
        self.write_mem(bus, addr.wrapping_add(1), lo);
    }

    /// Register-file byte `r` (through the full decode, so `r >= 0x80` reads 0).
    pub(crate) fn read_r8(&mut self, bus: &mut impl Bus, r: u8) -> u8 {
        self.read_mem(bus, u16::from(r))
    }

    pub(crate) fn write_r8(&mut self, bus: &mut impl Bus, r: u8, val: u8) {
        self.write_mem(bus, u16::from(r), val);
    }

    /// Register pair: high byte at `r - 1` (wrapping within 8 bits), low at `r`.
    pub(crate) fn read_r16(&mut self, bus: &mut impl Bus, r: u8) -> u16 {
        let hi = self.read_r8(bus, r.wrapping_sub(1));
        let lo = self.read_r8(bus, r);
        u16::from_be_bytes([hi, lo])
    }

    pub(crate) fn write_r16(&mut self, bus: &mut impl Bus, r: u8, val: u16) {
        let [hi, lo] = val.to_be_bytes();
        self.write_r8(bus, r.wrapping_sub(1), hi);
        self.write_r8(bus, r, lo);
    }

    /// Peripheral-file register `p`, addressed as `$0100 + p` on the
    /// ordinary map (MAME `read_p`).
    pub(crate) fn read_p(&mut self, bus: &mut impl Bus, p: u8) -> u8 {
        self.read_mem(bus, PERIPHERAL_FILE_BASE + u16::from(p))
    }

    pub(crate) fn write_p(&mut self, bus: &mut impl Bus, p: u8, val: u8) {
        self.write_mem(bus, PERIPHERAL_FILE_BASE + u16::from(p), val);
    }

    pub(crate) fn imm8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = self.read_mem(bus, self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    pub(crate) fn imm16(&mut self, bus: &mut impl Bus) -> u16 {
        let hi = self.imm8(bus);
        let lo = self.imm8(bus);
        u16::from_be_bytes([hi, lo])
    }

    pub(crate) fn push8(&mut self, bus: &mut impl Bus, val: u8) {
        self.sp = self.sp.wrapping_add(1);
        self.write_r8(bus, self.sp, val);
    }

    pub(crate) fn pull8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = self.read_r8(bus, self.sp);
        self.sp = self.sp.wrapping_sub(1);
        v
    }

    /// High byte first, so the low byte ends up on top.
    pub(crate) fn push16(&mut self, bus: &mut impl Bus, val: u16) {
        let [hi, lo] = val.to_be_bytes();
        self.push8(bus, hi);
        self.push8(bus, lo);
    }

    pub(crate) fn pull16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.pull8(bus);
        let hi = self.pull8(bus);
        u16::from_be_bytes([hi, lo])
    }
}
