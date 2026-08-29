//! Hardware (S) stack push/pull and the PSHS/PULS/PSHU/PULU register-mask
//! transfer, shared by subroutine calls, the interrupt frame in
//! [`crate::MC6809::enter_interrupt`], and the explicit stack opcodes in
//! [`crate::exec`].

use crate::{Bus, MC6809, PUSH_PULL_BASE_CYCLES, stack_mask};

impl MC6809 {
    /// Push PC (or any 16-bit value) onto the hardware (S) stack, big-endian.
    pub(crate) fn push16_s(&mut self, bus: &mut impl Bus, val: u16) {
        self.s = self.s.wrapping_sub(2);
        bus.write_u16(self.s, val);
    }

    /// Pull a 16-bit value from the hardware (S) stack.
    pub(crate) fn pull16_s(&mut self, bus: &mut impl Bus) -> u16 {
        let v = bus.read_u16(self.s);
        self.s = self.s.wrapping_add(2);
        v
    }

    /// PSHS/PSHU. `to_s` selects the S stack, otherwise U. Bit 6 in `mask` pushes
    /// the *other* stack pointer. Push order is PC, U/S, Y, X, DP, B, A, CC
    /// (highest address first). Returns the cycle count (base + 1 per byte).
    pub(crate) fn psh(&mut self, bus: &mut impl Bus, mask: u8, to_s: bool) -> u32 {
        // 16-bit push stores low byte first (higher address), leaving the value big-endian.
        let mut sp = if to_s { self.s } else { self.u };
        let other = if to_s { self.u } else { self.s };
        let mut push8 = |sp: &mut u16, v: u8, n: &mut u32| {
            *sp = sp.wrapping_sub(1);
            bus.write(*sp, v);
            *n += 1;
        };
        let mut bytes = 0u32;
        if mask & stack_mask::PC != 0 {
            push8(&mut sp, self.pc as u8, &mut bytes);
            push8(&mut sp, (self.pc >> 8) as u8, &mut bytes);
        }
        if mask & stack_mask::OTHER_STACK_PTR != 0 {
            push8(&mut sp, other as u8, &mut bytes);
            push8(&mut sp, (other >> 8) as u8, &mut bytes);
        }
        if mask & stack_mask::Y != 0 {
            push8(&mut sp, self.y as u8, &mut bytes);
            push8(&mut sp, (self.y >> 8) as u8, &mut bytes);
        }
        if mask & stack_mask::X != 0 {
            push8(&mut sp, self.x as u8, &mut bytes);
            push8(&mut sp, (self.x >> 8) as u8, &mut bytes);
        }
        if mask & stack_mask::DP != 0 {
            push8(&mut sp, self.dp, &mut bytes);
        }
        if mask & stack_mask::B != 0 {
            push8(&mut sp, self.b, &mut bytes);
        }
        if mask & stack_mask::A != 0 {
            push8(&mut sp, self.a, &mut bytes);
        }
        if mask & stack_mask::CC != 0 {
            push8(&mut sp, self.cc, &mut bytes);
        }
        if to_s {
            self.s = sp;
        } else {
            self.u = sp;
        }
        PUSH_PULL_BASE_CYCLES + bytes
    }

    /// PULS/PULU — inverse of [`Self::psh`]. Pull order is CC, A, B, DP, X, Y,
    /// U/S, PC. Bit 6 pulls the *other* stack pointer.
    pub(crate) fn pul(&mut self, bus: &mut impl Bus, mask: u8, from_s: bool) -> u32 {
        let mut sp = if from_s { self.s } else { self.u };
        let mut bytes = 0u32;
        let mut pull8 = |sp: &mut u16, n: &mut u32| {
            let v = bus.read(*sp);
            *sp = sp.wrapping_add(1);
            *n += 1;
            v
        };
        if mask & stack_mask::CC != 0 {
            self.cc = pull8(&mut sp, &mut bytes);
        }
        if mask & stack_mask::A != 0 {
            self.a = pull8(&mut sp, &mut bytes);
        }
        if mask & stack_mask::B != 0 {
            self.b = pull8(&mut sp, &mut bytes);
        }
        if mask & stack_mask::DP != 0 {
            self.dp = pull8(&mut sp, &mut bytes);
        }
        if mask & stack_mask::X != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            self.x = ((hi as u16) << 8) | lo as u16;
        }
        if mask & stack_mask::Y != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            self.y = ((hi as u16) << 8) | lo as u16;
        }
        if mask & stack_mask::OTHER_STACK_PTR != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            let v = ((hi as u16) << 8) | lo as u16;
            if from_s {
                self.u = v;
            } else {
                // PULU loading S is a program load and must arm NMI recognition
                // (see `nmi_armed`).
                self.load_s(v);
            }
        }
        if mask & stack_mask::PC != 0 {
            let hi = pull8(&mut sp, &mut bytes);
            let lo = pull8(&mut sp, &mut bytes);
            self.pc = ((hi as u16) << 8) | lo as u16;
        }
        if from_s {
            self.s = sp;
        } else {
            self.u = sp;
        }
        PUSH_PULL_BASE_CYCLES + bytes
    }
}
