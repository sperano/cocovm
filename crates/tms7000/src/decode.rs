//! Opcode map (MAME `execute_one`), expressed as the regular grid it is: an
//! addressing mode crossed with an operation for `$12-$7F`, `$80-$A7`, and
//! `$B0-$DF`, plus the irregular singles. `$B0` (MOV A,A, the documented
//! CLRC/TSTA) and `$B1` (MOV B,A, undocumented) are real instructions here
//! as in MAME; the 29 holes execute as [`Decoded::Illegal`].

/// Operand fetch/writeback shapes (MAME `am_*`). `X2y` reads `X`, applies
/// the op with `y` as the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    A,
    B,
    R,
    A2a,
    A2b,
    A2r,
    A2p,
    B2a,
    B2b,
    B2r,
    B2p,
    R2a,
    R2b,
    R2r,
    I2a,
    I2b,
    I2r,
    I2p,
    P2a,
    P2b,
}

/// Operations (MAME `op_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Clr,
    Dec,
    Inc,
    Inv,
    Rl,
    Rlc,
    Rr,
    Rrc,
    Swap,
    Xchb,
    Adc,
    Add,
    And,
    Cmp,
    Dac,
    Dsb,
    Mpy,
    Mov,
    Or,
    Sbb,
    Sub,
    Xor,
    Djnz,
    Btjo,
    Btjz,
}

/// Conditional-jump conditions for `$E0-$E7`, in opcode order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cond {
    Always,
    N,
    Z,
    C,
    /// `!(Z | N)`: JP.
    Positive,
    /// `!N`: JPZ.
    NotN,
    NotZ,
    NotC,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    Am(Mode, Op),
    Nop,
    Idle,
    Eint,
    Dint,
    Setc,
    PopSt,
    Stsp,
    Rets,
    Reti,
    Ldsp,
    PushSt,
    MovdDir,
    MovdInd,
    MovdInx,
    LdaDir,
    LdaInd,
    LdaInx,
    StaDir,
    StaInd,
    StaInx,
    BrDir,
    BrInd,
    BrInx,
    CmpaDir,
    CmpaInd,
    CmpaInx,
    CallDir,
    CallInd,
    CallInx,
    PushA,
    PushB,
    PushR,
    PopA,
    PopB,
    PopR,
    DecdA,
    DecdB,
    DecdR,
    Jmp(Cond),
    /// `TRAP n`, carrying the opcode (vector at `$FF00 | (op << 1)`).
    Trap(u8),
    Illegal,
}

/// Two-operand ops by low nibble for rows `$1x-$7x` (`0`/`1` are illegal).
const fn dyadic(low: u8) -> Op {
    match low {
        0x2 => Op::Mov,
        0x3 => Op::And,
        0x4 => Op::Or,
        0x5 => Op::Xor,
        0x6 => Op::Btjo,
        0x7 => Op::Btjz,
        0x8 => Op::Add,
        0x9 => Op::Adc,
        0xA => Op::Sub,
        0xB => Op::Sbb,
        0xC => Op::Mpy,
        0xD => Op::Cmp,
        0xE => Op::Dac,
        0xF => Op::Dsb,
        _ => panic!("dyadic rows start at low nibble 2"),
    }
}

/// Single-operand ops by low nibble for rows `$Bx-$Dx` (`8`/`9`/`B` are
/// push/pop/decd, `0`/`1` are the MOV aliases).
const fn monadic(low: u8) -> Op {
    match low {
        0x2 => Op::Dec,
        0x3 => Op::Inc,
        0x4 => Op::Inv,
        0x5 => Op::Clr,
        0x6 => Op::Xchb,
        0x7 => Op::Swap,
        0xA => Op::Djnz,
        0xC => Op::Rr,
        0xD => Op::Rrc,
        0xE => Op::Rl,
        0xF => Op::Rlc,
        _ => panic!("not a monadic slot"),
    }
}

const fn decode(op: u8) -> Decoded {
    let low = op & 0x0F;
    match op {
        0x00 => Decoded::Nop,
        0x01 => Decoded::Idle,
        0x05 => Decoded::Eint,
        0x06 => Decoded::Dint,
        0x07 => Decoded::Setc,
        0x08 => Decoded::PopSt,
        0x09 => Decoded::Stsp,
        0x0A => Decoded::Rets,
        0x0B => Decoded::Reti,
        0x0D => Decoded::Ldsp,
        0x0E => Decoded::PushSt,

        0x12..=0x1F => Decoded::Am(Mode::R2a, dyadic(low)),
        0x22..=0x2F => Decoded::Am(Mode::I2a, dyadic(low)),
        0x32..=0x3F => Decoded::Am(Mode::R2b, dyadic(low)),
        0x42..=0x4F => Decoded::Am(Mode::R2r, dyadic(low)),
        0x52..=0x5F => Decoded::Am(Mode::I2b, dyadic(low)),
        0x62..=0x6F => Decoded::Am(Mode::B2a, dyadic(low)),
        0x72..=0x7F => Decoded::Am(Mode::I2r, dyadic(low)),

        0x80 => Decoded::Am(Mode::P2a, Op::Mov),
        0x82..=0x87 => Decoded::Am(Mode::A2p, dyadic(low)),
        0x88 => Decoded::MovdDir,
        0x8A => Decoded::LdaDir,
        0x8B => Decoded::StaDir,
        0x8C => Decoded::BrDir,
        0x8D => Decoded::CmpaDir,
        0x8E => Decoded::CallDir,

        0x91 => Decoded::Am(Mode::P2b, Op::Mov),
        0x92..=0x97 => Decoded::Am(Mode::B2p, dyadic(low)),
        0x98 => Decoded::MovdInd,
        0x9A => Decoded::LdaInd,
        0x9B => Decoded::StaInd,
        0x9C => Decoded::BrInd,
        0x9D => Decoded::CmpaInd,
        0x9E => Decoded::CallInd,

        0xA2..=0xA7 => Decoded::Am(Mode::I2p, dyadic(low)),
        0xA8 => Decoded::MovdInx,
        0xAA => Decoded::LdaInx,
        0xAB => Decoded::StaInx,
        0xAC => Decoded::BrInx,
        0xAD => Decoded::CmpaInx,
        0xAE => Decoded::CallInx,

        0xB0 => Decoded::Am(Mode::A2a, Op::Mov),
        0xB1 => Decoded::Am(Mode::B2a, Op::Mov),
        0xB8 => Decoded::PushA,
        0xB9 => Decoded::PopA,
        0xBB => Decoded::DecdA,
        0xB2..=0xBF => Decoded::Am(Mode::A, monadic(low)),

        0xC0 => Decoded::Am(Mode::A2b, Op::Mov),
        0xC1 => Decoded::Am(Mode::B2b, Op::Mov),
        0xC8 => Decoded::PushB,
        0xC9 => Decoded::PopB,
        0xCB => Decoded::DecdB,
        0xC2..=0xCF => Decoded::Am(Mode::B, monadic(low)),

        0xD0 => Decoded::Am(Mode::A2r, Op::Mov),
        0xD1 => Decoded::Am(Mode::B2r, Op::Mov),
        0xD8 => Decoded::PushR,
        0xD9 => Decoded::PopR,
        0xDB => Decoded::DecdR,
        0xD2..=0xDF => Decoded::Am(Mode::R, monadic(low)),

        0xE0 => Decoded::Jmp(Cond::Always),
        0xE1 => Decoded::Jmp(Cond::N),
        0xE2 => Decoded::Jmp(Cond::Z),
        0xE3 => Decoded::Jmp(Cond::C),
        0xE4 => Decoded::Jmp(Cond::Positive),
        0xE5 => Decoded::Jmp(Cond::NotN),
        0xE6 => Decoded::Jmp(Cond::NotZ),
        0xE7 => Decoded::Jmp(Cond::NotC),
        0xE8..=0xFF => Decoded::Trap(op),

        _ => Decoded::Illegal,
    }
}

/// The full opcode map.
pub(crate) const TABLE: [Decoded; 256] = {
    let mut table = [Decoded::Illegal; 256];
    let mut op = 0;
    while op < 256 {
        table[op] = decode(op as u8);
        op += 1;
    }
    table
};
