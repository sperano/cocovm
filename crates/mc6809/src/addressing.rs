//! Effective-address computation and instruction-stream fetching: the
//! addressing-mode machinery shared by every opcode family in [`crate::exec`].

use crate::{postbyte, Bus, MC6809};

impl MC6809 {
    pub(crate) fn fetch_u8(&mut self, bus: &mut impl Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    pub(crate) fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
        let hi = self.fetch_u8(bus) as u16;
        let lo = self.fetch_u8(bus) as u16;
        (hi << 8) | lo
    }

    /// Direct-mode effective address: `DP:operand_byte`.
    pub(crate) fn ea_direct(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch_u8(bus) as u16;
        ((self.dp as u16) << 8) | lo
    }

    /// Extended-mode effective address: a 16-bit operand.
    pub(crate) fn ea_extended(&mut self, bus: &mut impl Bus) -> u16 {
        self.fetch_u16(bus)
    }

    /// One of the four index registers selected by a 2-bit postbyte field
    /// (00=X, 01=Y, 10=U, 11=S).
    fn index_reg(&self, sel: u8) -> u16 {
        match sel & 0b11 {
            0b00 => self.x,
            0b01 => self.y,
            0b10 => self.u,
            _ => self.s,
        }
    }

    fn set_index_reg(&mut self, sel: u8, val: u16) {
        match sel & 0b11 {
            0b00 => self.x = val,
            0b01 => self.y = val,
            0b10 => self.u = val,
            _ => self.load_s(val),
        }
    }

    /// Decode an indexed-addressing postbyte and return the effective address and
    /// the *extra* cycles it costs (added to the instruction's indexed base cost).
    /// A large fraction of instructions route through here (see `DESIGN.md` §5), so
    /// it aims to be a faithful transcription of the datasheet's indexed-mode table.
    ///
    /// Postbyte layout when bit 7 is set: `1 rr i mmmm` — `rr` selects the register,
    /// `i` is the indirect bit, `mmmm` the sub-mode. When bit 7 is clear the whole
    /// low 5 bits are a signed offset (`0 rr nnnnn`), which has no indirect form.
    pub(crate) fn ea_indexed(&mut self, bus: &mut impl Bus) -> (u16, u32) {
        let pb = self.fetch_u8(bus);

        // 5-bit signed constant offset — the only non-indirect-capable form.
        if pb & 0x80 == 0 {
            return self.ea_indexed_offset5(pb);
        }

        self.ea_indexed_full(bus, pb)
    }

    /// The `0 rr nnnnn` postbyte form: a 5-bit signed constant offset, never
    /// indirectable.
    fn ea_indexed_offset5(&mut self, pb: u8) -> (u16, u32) {
        use postbyte::*;
        let reg = self.index_reg(pb >> REG_SHIFT);
        let n = pb & OFFSET5_MASK;
        let offset = if n & OFFSET5_SIGN != 0 {
            n as i16 - (OFFSET5_SIGN as i16 * 2)
        } else {
            n as i16
        };
        (reg.wrapping_add(offset as u16), 1)
    }

    /// The `1 rr i mmmm` postbyte form: dispatch by sub-mode, then apply the
    /// indirect fetch (`[...]`) if the indirect bit is set.
    fn ea_indexed_full(&mut self, bus: &mut impl Bus, pb: u8) -> (u16, u32) {
        use postbyte::*;
        let sel = pb >> REG_SHIFT;
        let indirect = pb & INDIRECT != 0;
        let (mut ea, mut extra) = self.ea_indexed_submode(bus, sel, pb & MODE_MASK);

        if indirect {
            ea = bus.read_u16(ea);
            extra += INDIRECT_CYCLES;
        }
        (ea, extra)
    }

    /// The sub-mode (`mmmm`) field of an indexed postbyte: which register,
    /// auto-increment/decrement, or offset form to use.
    fn ea_indexed_submode(&mut self, bus: &mut impl Bus, sel: u8, mode: u8) -> (u16, u32) {
        match mode {
            0b0000 => {
                // ,R+  (auto-increment by 1; not indirectable on real silicon)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(1));
                (r, 2)
            }
            0b0001 => {
                // ,R++  (auto-increment by 2)
                let r = self.index_reg(sel);
                self.set_index_reg(sel, r.wrapping_add(2));
                (r, 3)
            }
            0b0010 => {
                // ,-R  (auto-decrement by 1; not indirectable on real silicon)
                let r = self.index_reg(sel).wrapping_sub(1);
                self.set_index_reg(sel, r);
                (r, 2)
            }
            0b0011 => {
                // ,--R  (auto-decrement by 2)
                let r = self.index_reg(sel).wrapping_sub(2);
                self.set_index_reg(sel, r);
                (r, 3)
            }
            0b0100 => (self.index_reg(sel), 0), // ,R  (no offset)
            0b0101 => {
                // B,R  (signed accumulator offset)
                let ofs = self.b as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b0110 => {
                // A,R
                let ofs = self.a as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1000 => {
                // n,R  (8-bit signed offset)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.index_reg(sel).wrapping_add(ofs), 1)
            }
            0b1001 => {
                // n,R  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1011 => {
                // D,R  (16-bit accumulator offset)
                let ofs = self.d();
                (self.index_reg(sel).wrapping_add(ofs), 4)
            }
            0b1100 => {
                // n,PCR  (8-bit signed offset from the *next* instruction)
                let ofs = self.fetch_u8(bus) as i8 as i16 as u16;
                (self.pc.wrapping_add(ofs), 1)
            }
            0b1101 => {
                // n,PCR  (16-bit offset)
                let ofs = self.fetch_u16(bus);
                (self.pc.wrapping_add(ofs), 5)
            }
            0b1111 => {
                // [n]  extended indirect (register field ignored). The base cost
                // here plus the indirect fetch below sum to the datasheet's 5.
                (self.fetch_u16(bus), 2)
            }
            // Reserved/illegal postbytes (0b0111, 0b1010, 0b1110): behaviour is
            // undefined on hardware; fall back to a plain register read.
            _ => (self.index_reg(sel), 0),
        }
    }

    pub(crate) fn read_direct8(&mut self, bus: &mut impl Bus) -> u8 {
        let ea = self.ea_direct(bus);
        bus.read(ea)
    }

    pub(crate) fn read_extended8(&mut self, bus: &mut impl Bus) -> u8 {
        let ea = self.ea_extended(bus);
        bus.read(ea)
    }

    pub(crate) fn read_direct16(&mut self, bus: &mut impl Bus) -> u16 {
        let ea = self.ea_direct(bus);
        bus.read_u16(ea)
    }

    pub(crate) fn read_extended16(&mut self, bus: &mut impl Bus) -> u16 {
        let ea = self.ea_extended(bus);
        bus.read_u16(ea)
    }
}
