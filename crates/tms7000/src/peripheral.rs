//! Peripheral file (`$0100-$010B`), the external interrupt lines, and
//! interrupt dispatch (MAME `tms7000_pf_r/w`, `execute_set_input`,
//! `flag_ext_interrupt`, `check_interrupts`, `do_interrupt`).

use crate::{Bus, Port, TMS7040, VECTOR_INT1, st};

/// Peripheral-file register offsets on the 70x0 family.
pub(crate) mod pf {
    pub const IOCNT0: u8 = 0x00;
    pub const T1DATA: u8 = 0x02;
    pub const T1CTL: u8 = 0x03;
    pub const APORT: u8 = 0x04;
    pub const ADDR: u8 = 0x05;
    pub const BPORT: u8 = 0x06;
    pub const CPORT: u8 = 0x08;
    pub const CDDR: u8 = 0x09;
    pub const DPORT: u8 = 0x0A;
    pub const DDDR: u8 = 0x0B;
}

/// IOCNT0 layout: enable bits at d0/d2/d4, flag bits at d1/d3/d5 for
/// INT1/INT2/INT3. Writing a 1 to a flag bit clears it; enables and the
/// (unimplemented) memory-mode bits d6-d7 are written through.
mod iocnt0 {
    pub const FLAGS: u8 = 0x2A;
    pub const WRITE_THROUGH: u8 = 0xD5;
    pub const INT2_FLAG: u8 = 0x08;
}

/// External line indices into [`TMS7040::int_line`] (MAME
/// `TMS7000_INT1_LINE` = 0, `TMS7000_INT3_LINE` = 1).
const INT1_LINE: usize = 0;
const INT3_LINE: usize = 1;

/// Cycles to enter an interrupt handler, and from IDLE (MAME `do_interrupt`).
const INTERRUPT_CYCLES: u32 = 19;
const INTERRUPT_FROM_IDLE_CYCLES: u32 = 17;

impl TMS7040 {
    /// Drive the INT1 pin (on the SSC: the SP0256's load request).
    pub fn set_int1(&mut self, level: bool) {
        self.set_ext_line(INT1_LINE, level);
    }

    /// Drive the INT3 pin (on the SSC: a host byte was latched).
    pub fn set_int3(&mut self, level: bool) {
        self.set_ext_line(INT3_LINE, level);
    }

    /// MAME `execute_set_input`: level tracked into the IOCNT0 flag; a
    /// rising INT3 also captures the timer. Dispatch waits for the next
    /// [`Self::step`].
    fn set_ext_line(&mut self, line: usize, level: bool) {
        if self.int_line[line] == level {
            return;
        }
        self.int_line[line] = level;
        self.flag_ext_interrupt(line);
        if level && line == INT3_LINE {
            self.timer1.capture();
        }
    }

    /// Mirror an external line's level into its IOCNT0 flag bit.
    fn flag_ext_interrupt(&mut self, line: usize) {
        let flag = 0x02 << (4 * line);
        if self.int_line[line] {
            self.io_control |= flag;
        } else {
            self.io_control &= !flag;
        }
    }

    /// Timer 1 underflow: raise the INT2 flag.
    pub(crate) fn flag_timer_interrupt(&mut self) {
        self.io_control |= iocnt0::INT2_FLAG;
    }

    /// Take the highest-priority enabled, flagged interrupt (INT1 > INT2 >
    /// INT3) if the global enable is set. Returns the line taken.
    ///
    /// Called before every instruction, where MAME checks once per timeslice
    /// plus on line changes, IOCNT0 writes, EINT, RETI and POP ST. Those
    /// in-instruction checks dispatch after the operands are fetched and the
    /// cycles charged, so the pushed PC and the cycle total come out the
    /// same as dispatching before the next instruction.
    pub(crate) fn check_interrupts(&mut self, bus: &mut impl Bus) -> Option<u8> {
        if self.st & st::I == 0 {
            return None;
        }
        for irq in 0..3u8 {
            let shift = irq * 2;
            if (self.io_control >> shift) & 3 != 3 {
                continue;
            }
            // Ack — then re-flag at once if the external line is still high.
            self.io_control &= !(0x02 << shift);
            match irq {
                0 => self.flag_ext_interrupt(INT1_LINE),
                2 => self.flag_ext_interrupt(INT3_LINE),
                _ => {}
            }
            self.do_interrupt(bus, irq);
            return Some(irq + 1);
        }
        None
    }

    fn do_interrupt(&mut self, bus: &mut impl Bus, irq: u8) {
        if self.idle {
            self.burn(INTERRUPT_FROM_IDLE_CYCLES);
            self.pc = self.pc.wrapping_add(1);
            self.idle = false;
        } else {
            self.burn(INTERRUPT_CYCLES);
        }
        self.push8(bus, self.st);
        self.push16(bus, self.pc);
        self.st = 0;
        self.pc = self.read_mem16(bus, VECTOR_INT1 - u16::from(irq) * 2);
    }

    // ---- Peripheral file --------------------------------------------------

    pub(crate) fn pf_read(&mut self, bus: &mut impl Bus, offset: u8) -> u8 {
        match offset {
            pf::IOCNT0 => self.io_control,
            pf::T1DATA => self.timer1.decrementer(),
            pf::T1CTL => self.timer1.capture_latch(),
            pf::APORT | pf::BPORT | pf::CPORT | pf::DPORT => {
                let port = Port::ALL[usize::from(offset / 2 - 2)];
                let ddr = self.port_ddr[port.index()];
                (bus.read_port(port) & !ddr) | (self.port_latch[port.index()] & ddr)
            }
            pf::ADDR | pf::CDDR | pf::DDDR => self.port_ddr[usize::from(offset / 2 - 2)],
            _ => 0,
        }
    }

    pub(crate) fn pf_write(&mut self, bus: &mut impl Bus, offset: u8, val: u8) {
        match offset {
            pf::IOCNT0 => {
                self.io_control =
                    (self.io_control & (!val & iocnt0::FLAGS)) | (val & iocnt0::WRITE_THROUGH);
                // A cleared flag springs back if its line is still high.
                if val & 0x02 != 0 {
                    self.flag_ext_interrupt(INT1_LINE);
                }
                if val & 0x20 != 0 {
                    self.flag_ext_interrupt(INT3_LINE);
                }
            }
            pf::T1DATA => self.timer1.write_data(val),
            pf::T1CTL => self.timer1.write_control(val),
            // The 70x0 has no port A output latch and no DDR A (MAME
            // `tms7000_mem`: `map(0x0104, 0x0105).nopw()`).
            pf::APORT | pf::ADDR => {}
            pf::BPORT | pf::CPORT | pf::DPORT => {
                let port = Port::ALL[usize::from(offset / 2 - 2)];
                bus.write_port(port, val & self.port_ddr[port.index()]);
                self.port_latch[port.index()] = val;
            }
            // Changing a DDR does not refresh the pins.
            pf::CDDR | pf::DDDR => self.port_ddr[usize::from(offset / 2 - 2)] = val,
            _ => {}
        }
    }

    /// Side-effect-free peripheral-file read for debuggers.
    pub(crate) fn pf_peek(&self, offset: u8) -> u8 {
        match offset {
            pf::IOCNT0 => self.io_control,
            pf::T1DATA => self.timer1.decrementer(),
            pf::T1CTL => self.timer1.capture_latch(),
            pf::APORT | pf::BPORT | pf::CPORT | pf::DPORT => {
                self.port_latch[usize::from(offset / 2 - 2)]
            }
            pf::ADDR | pf::CDDR | pf::DDDR => self.port_ddr[usize::from(offset / 2 - 2)],
            _ => 0,
        }
    }
}
