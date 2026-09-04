//! The addressing-mode grid: fetch operands per [`Mode`], charge the mode's
//! base cost, apply the [`Op`], write back if it produced a value (MAME
//! `am_*` and the `AM_WB` macro). Immediate bytes are fetched in MAME's
//! order — source before destination.

use crate::decode::{Mode, Op};
use crate::{Bus, TMS7040};

/// Where an op's result goes.
#[derive(Clone, Copy)]
pub(super) enum Dest {
    /// Register file entry (A = 0, B = 1).
    Rf(u8),
    /// Peripheral file register.
    Pf(u8),
}

/// Base cycle cost per mode (MAME `am_*`: the first `m_icount -=`).
fn base_cycles(mode: Mode) -> u32 {
    match mode {
        Mode::A | Mode::B | Mode::B2a => 5,
        Mode::A2a | Mode::A2b | Mode::B2b => 6,
        Mode::R | Mode::B2r | Mode::I2a | Mode::I2b => 7,
        Mode::A2r | Mode::R2a | Mode::R2b | Mode::P2b => 8,
        Mode::B2p | Mode::I2r | Mode::P2a => 9,
        Mode::A2p | Mode::R2r => 10,
        Mode::I2p => 11,
    }
}

impl TMS7040 {
    pub(super) fn exec_am(&mut self, bus: &mut impl Bus, mode: Mode, op: Op) {
        self.burn(base_cycles(mode));
        let (dest, p1, p2) = self.fetch(bus, mode);
        if let Some(result) = self.apply(bus, op, p1, p2) {
            match dest {
                Dest::Rf(r) => self.write_r8(bus, r, result),
                Dest::Pf(p) => self.write_p(bus, p, result),
            }
        }
    }

    /// `(destination, param1 = destination's current value, param2 = source)`.
    fn fetch(&mut self, bus: &mut impl Bus, mode: Mode) -> (Dest, u8, u8) {
        let a = self.rf[0];
        let b = self.rf[1];
        match mode {
            Mode::A => (Dest::Rf(0), a, 0),
            Mode::B => (Dest::Rf(1), b, 0),
            Mode::R => {
                let r = self.imm8(bus);
                (Dest::Rf(r), self.read_r8(bus, r), 0)
            }
            Mode::A2a => (Dest::Rf(0), a, a),
            Mode::A2b => (Dest::Rf(1), b, a),
            Mode::A2p => {
                let p = self.imm8(bus);
                (Dest::Pf(p), self.read_p(bus, p), a)
            }
            Mode::A2r => {
                let r = self.imm8(bus);
                (Dest::Rf(r), self.read_r8(bus, r), a)
            }
            Mode::B2a => (Dest::Rf(0), a, b),
            Mode::B2b => (Dest::Rf(1), b, b),
            Mode::B2r => {
                let r = self.imm8(bus);
                (Dest::Rf(r), self.read_r8(bus, r), b)
            }
            Mode::B2p => {
                let p = self.imm8(bus);
                (Dest::Pf(p), self.read_p(bus, p), b)
            }
            Mode::R2a => {
                let src = self.imm8(bus);
                (Dest::Rf(0), a, self.read_r8(bus, src))
            }
            Mode::R2b => {
                let src = self.imm8(bus);
                (Dest::Rf(1), b, self.read_r8(bus, src))
            }
            Mode::R2r => {
                let src = self.imm8(bus);
                let p2 = self.read_r8(bus, src);
                let r = self.imm8(bus);
                (Dest::Rf(r), self.read_r8(bus, r), p2)
            }
            Mode::I2a => {
                let imm = self.imm8(bus);
                (Dest::Rf(0), a, imm)
            }
            Mode::I2b => {
                let imm = self.imm8(bus);
                (Dest::Rf(1), b, imm)
            }
            Mode::I2r => {
                let imm = self.imm8(bus);
                let r = self.imm8(bus);
                (Dest::Rf(r), self.read_r8(bus, r), imm)
            }
            Mode::I2p => {
                let imm = self.imm8(bus);
                let p = self.imm8(bus);
                (Dest::Pf(p), self.read_p(bus, p), imm)
            }
            Mode::P2a => {
                let p = self.imm8(bus);
                (Dest::Rf(0), a, self.read_p(bus, p))
            }
            Mode::P2b => {
                let p = self.imm8(bus);
                (Dest::Rf(1), b, self.read_p(bus, p))
            }
        }
    }
}
