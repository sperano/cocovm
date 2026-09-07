//! The chip's own address decode (MAME `tms7040_mem`, extended here with
//! IOCNT0's memory-mode bits, which MAME does not implement) and the
//! byte/word, immediate, and stack accessors the opcodes are written
//! against (MAME `tms7000.h` inline helpers).

use crate::{Bus, PERIPHERAL_FILE_BASE, ROM_BASE, TMS7040};

/// Top of the register file; `$0080-$00FF` is unmapped on the 70x0 family
/// (reads 0, writes ignored — MAME `tms7000_unmapped_rf_r/w`), Reserved in
/// every memory mode (SPND001C Table 3-6) so this doesn't vary with mode.
const REGISTER_FILE_END: u16 = 0x007F;
const UNMAPPED_RF_END: u16 = 0x00FF;
/// Last peripheral-file register on the 70x0 family in Single-Chip mode;
/// Port C's (both expansion modes) and Port D's (Full-Expansion only)
/// registers are repurposed off-chip past [`CPORT_PF_ADDR`].
const PERIPHERAL_FILE_END: u16 = 0x010B;
/// `$0108`/`$0109`: CPORT/CDDR's peripheral-file addresses, mapped external
/// in both expansion modes (SPND001C 3.3.2 Note 1: "The Port C
/// Data-Direction Register is mapped into external memory").
const CPORT_PF_ADDR: u16 = PERIPHERAL_FILE_BASE + 0x08;
const CDDR_PF_ADDR: u16 = PERIPHERAL_FILE_BASE + 0x09;
/// `$010A`/`$010B`: DPORT/DDDR's peripheral-file addresses, mapped external
/// in Full-Expansion only, which outputs the address MSB on Port D instead
/// (SPND001C 3.3.3).
const DPORT_PF_ADDR: u16 = PERIPHERAL_FILE_BASE + 0x0A;
const DDDR_PF_ADDR: u16 = PERIPHERAL_FILE_BASE + 0x0B;
/// `$010C-$01FF`: off-chip peripheral expansion, wired in Peripheral- and
/// Full-Expansion modes, Not Available in Single-Chip (SPND001C Table 3-6).
const PERIPHERAL_EXPANSION_START: u16 = PERIPHERAL_FILE_END + 1;
const PERIPHERAL_EXPANSION_END: u16 = 0x01FF;
/// `$0200-$EFFF`: off-chip memory expansion, Full-Expansion mode only
/// (SPND001C Table 3-6; Figure 3-10 misprints this range as Not Available).
const MEMORY_EXPANSION_START: u16 = 0x0200;
const MEMORY_EXPANSION_END: u16 = 0xEFFF;
/// Value returned for a read the current memory mode makes Not Available
/// (SPND001C: "an undefined value is returned"); `0xFF` is this emulator's
/// convention for that undefined value, not a manual-specified one. Writes
/// to such a range are simply dropped.
const NOT_AVAILABLE_READ: u8 = 0xFF;

/// IOCNT0 bits 7:6 (SPND001C Table 3-5, Figure 3-16).
const MEMORY_MODE_SHIFT: u8 = 6;

/// Memory-mode select, IOCNT0 bits 7:6. `Undefined` (`11`) has no meaning
/// in the manual; treated as `SingleChip`, the mode IOCNT0 resets into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MemoryMode {
    SingleChip,
    PeripheralExpansion,
    FullExpansion,
}

/// Where an address routes under the current memory mode.
enum Decode {
    RegisterFile,
    Reserved,
    /// Peripheral-file offset (`addr - PERIPHERAL_FILE_BASE`).
    PeripheralFile(u8),
    NotAvailable,
    External,
    Rom,
}

impl TMS7040 {
    fn memory_mode(&self) -> MemoryMode {
        match self.io_control >> MEMORY_MODE_SHIFT {
            0b01 => MemoryMode::PeripheralExpansion,
            0b10 => MemoryMode::FullExpansion,
            _ => MemoryMode::SingleChip, // 0b00, and 0b11 Undefined
        }
    }

    /// SPND001C Table 3-6, `$0000-$FFFF` for the TMS70x0 family (Port C
    /// off-chip in both expansion modes, Port D only in Full-Expansion).
    fn decode(&self, addr: u16) -> Decode {
        match addr {
            0..=REGISTER_FILE_END => Decode::RegisterFile,
            0x0080..=UNMAPPED_RF_END => Decode::Reserved,
            PERIPHERAL_FILE_BASE..=PERIPHERAL_FILE_END => {
                let mode = self.memory_mode();
                let off_chip = match addr {
                    CPORT_PF_ADDR | CDDR_PF_ADDR => mode != MemoryMode::SingleChip,
                    DPORT_PF_ADDR | DDDR_PF_ADDR => mode == MemoryMode::FullExpansion,
                    _ => false,
                };
                if off_chip {
                    Decode::External
                } else {
                    Decode::PeripheralFile((addr - PERIPHERAL_FILE_BASE) as u8)
                }
            }
            PERIPHERAL_EXPANSION_START..=PERIPHERAL_EXPANSION_END => match self.memory_mode() {
                MemoryMode::SingleChip => Decode::NotAvailable,
                MemoryMode::PeripheralExpansion | MemoryMode::FullExpansion => Decode::External,
            },
            MEMORY_EXPANSION_START..=MEMORY_EXPANSION_END => match self.memory_mode() {
                MemoryMode::FullExpansion => Decode::External,
                MemoryMode::SingleChip | MemoryMode::PeripheralExpansion => Decode::NotAvailable,
            },
            ROM_BASE..=0xFFFF => Decode::Rom,
        }
    }

    pub(crate) fn rom_byte(&self, addr: u16) -> u8 {
        self.rom
            .get(usize::from(addr - ROM_BASE))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn read_mem(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        match self.decode(addr) {
            Decode::RegisterFile => self.rf[usize::from(addr)],
            Decode::Reserved => 0,
            Decode::PeripheralFile(offset) => self.pf_read(bus, offset),
            Decode::NotAvailable => NOT_AVAILABLE_READ,
            Decode::External => bus.read_ext(addr),
            Decode::Rom => self.rom_byte(addr),
        }
    }

    pub(crate) fn write_mem(&mut self, bus: &mut impl Bus, addr: u16, val: u8) {
        match self.decode(addr) {
            Decode::RegisterFile => self.rf[usize::from(addr)] = val,
            Decode::Reserved => {}
            Decode::PeripheralFile(offset) => self.pf_write(bus, offset, val),
            Decode::NotAvailable => {}
            Decode::External => bus.write_ext(addr, val),
            Decode::Rom => {}
        }
    }

    /// Side-effect-free version of [`Self::read_mem`], mirroring
    /// [`Self::decode`] without touching `Bus` — used by [`TMS7040::peek`].
    pub(crate) fn peek_mem(&self, addr: u16) -> u8 {
        match self.decode(addr) {
            Decode::RegisterFile => self.rf[usize::from(addr)],
            Decode::Reserved => 0,
            Decode::PeripheralFile(offset) => self.pf_peek(offset),
            Decode::NotAvailable => NOT_AVAILABLE_READ,
            Decode::External => 0,
            Decode::Rom => self.rom_byte(addr),
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
