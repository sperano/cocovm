//! MAME `7000dasm.cpp`'s `of[]` operand formats and `opcs[]` opcode table,
//! transcribed. Each format is a list of `(prefix, argument, suffix)`
//! tokens; MAME's printf templates map onto [`Arg`] one for one.

/// One operand argument and how it prints.
#[derive(Clone, Copy)]
pub(super) enum Arg {
    /// No byte consumed.
    None,
    /// One byte, decimal (`R%u`, `P%u`).
    Dec,
    /// One byte, uppercase hex without padding (`%X`).
    Hex,
    /// One byte, two-digit hex (`%02X`).
    Hex2,
    /// Two bytes big-endian, four-digit hex (`%04X`).
    Hex4,
    /// One signed byte; prints the absolute target `>XXXX` relative to the
    /// position after the byte.
    Rel,
    /// Two bytes, printed as an absolute `>XXXX` target.
    Abs,
}

pub(super) type Token = (&'static str, Arg, &'static str);

/// MAME's 46 operand formats, in `of[]` order. The leading space MAME
/// prints between mnemonic and operand is dropped here.
#[rustfmt::skip]
pub(super) const FORMATS: [&[Token]; 46] = [
    /*  0 */ &[("B,A", Arg::None, "")],
    /*  1 */ &[("R", Arg::Dec, ",A")],
    /*  2 */ &[("R", Arg::Dec, ",B")],
    /*  3 */ &[("R", Arg::Dec, ""), (",R", Arg::Dec, "")],
    /*  4 */ &[("%>", Arg::Hex, ",A")],
    /*  5 */ &[("%>", Arg::Hex, ",B")],
    /*  6 */ &[("%>", Arg::Hex, ""), (",R", Arg::Dec, "")],
    /*  7 */ &[("A,P", Arg::Dec, "")],
    /*  8 */ &[("B,P", Arg::Dec, "")],
    /*  9 */ &[("%>", Arg::Hex2, ""), (",P", Arg::Dec, "")],
    /* 10 */ &[("@>", Arg::Hex4, "")],
    /* 11 */ &[("R", Arg::Dec, "")],
    /* 12 */ &[("@>", Arg::Hex4, "(B)")],
    /* 13 */ &[("B,A,", Arg::Rel, "")],
    /* 14 */ &[("R", Arg::Dec, ",A"), (",", Arg::Rel, "")],
    /* 15 */ &[("R", Arg::Dec, ",B"), (",", Arg::Rel, "")],
    /* 16 */ &[("R", Arg::Dec, ""), (",R", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 17 */ &[("%>", Arg::Hex, ",A,"), ("", Arg::Rel, "")],
    /* 18 */ &[("%>", Arg::Hex, ",B,"), ("", Arg::Rel, "")],
    /* 19 */ &[("%>", Arg::Hex, ""), (",R", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 20 */ &[("A,P", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 21 */ &[("B,P", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 22 */ &[("%>", Arg::Hex2, ""), (",P", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 23 */ &[],
    /* 24 */ &[("R", Arg::Dec, "")],
    /* 25 */ &[("A,", Arg::Rel, "")],
    /* 26 */ &[("B,", Arg::Rel, "")],
    /* 27 */ &[("R", Arg::Dec, ""), (",", Arg::Rel, "")],
    /* 28 */ &[("", Arg::Rel, "")],
    /* 29 */ &[("A,B", Arg::None, "")],
    /* 30 */ &[("B,A", Arg::None, "")],
    /* 31 */ &[("A,R", Arg::Dec, "")],
    /* 32 */ &[("B,R", Arg::Dec, "")],
    /* 33 */ &[("R", Arg::Dec, ",A")],
    /* 34 */ &[("R", Arg::Dec, ",B")],
    /* 35 */ &[("R", Arg::Dec, ""), (",R", Arg::Dec, "")],
    /* 36 */ &[("%>", Arg::Hex, ",A")],
    /* 37 */ &[("%>", Arg::Hex, ",B")],
    /* 38 */ &[("%>", Arg::Hex, ""), (",R", Arg::Dec, "")],
    /* 39 */ &[("%>", Arg::Hex4, ""), (",R", Arg::Dec, "")],
    /* 40 */ &[("%>", Arg::Hex4, "(B)"), (",R", Arg::Dec, "")],
    /* 41 */ &[("P", Arg::Dec, ",A")],
    /* 42 */ &[("P", Arg::Dec, ",B")],
    /* 43 */ &[("", Arg::Abs, "")],
    /* 44 */ &[],
    /* 45 */ &[("*R", Arg::Dec, "")],
];

/// `(opcode, mnemonic, format index)` in MAME `opcs[]` order; the first
/// entry for an opcode wins (`$B0` is `CLRC`, not `TSTA`). `$B1` is not in
/// MAME's table (its disassembler doesn't name the opcode), added here as
/// `MOV B,A` to match `execute_one`'s undocumented handling and this crate's
/// own `decode.rs`.
#[rustfmt::skip]
pub(super) const OPCODES: &[(u8, &str, usize)] = &[
    (0x69, "ADC", 0), (0x19, "ADC", 1), (0x39, "ADC", 2), (0x49, "ADC", 3),
    (0x29, "ADC", 4), (0x59, "ADC", 5), (0x79, "ADC", 6),
    (0x68, "ADD", 0), (0x18, "ADD", 1), (0x38, "ADD", 2), (0x48, "ADD", 3),
    (0x28, "ADD", 4), (0x58, "ADD", 5), (0x78, "ADD", 6),
    (0x63, "AND", 0), (0x13, "AND", 1), (0x33, "AND", 2), (0x43, "AND", 3),
    (0x23, "AND", 4), (0x53, "AND", 5), (0x73, "AND", 6),
    (0x83, "ANDP", 7), (0x93, "ANDP", 8), (0xA3, "ANDP", 9),
    (0x8C, "BR", 43), (0x9C, "BR", 45), (0xAC, "BR", 12),
    (0x66, "BTJO", 13), (0x16, "BTJO", 14), (0x36, "BTJO", 15), (0x46, "BTJO", 16),
    (0x26, "BTJO", 17), (0x56, "BTJO", 18), (0x76, "BTJO", 19),
    (0x86, "BTJOP", 20), (0x96, "BTJOP", 21), (0xA6, "BTJOP", 22),
    (0x67, "BTJZ", 13), (0x17, "BTJZ", 14), (0x37, "BTJZ", 15), (0x47, "BTJZ", 16),
    (0x27, "BTJZ", 17), (0x57, "BTJZ", 18), (0x77, "BTJZ", 19),
    (0x87, "BTJZP", 20), (0x97, "BTJZP", 21), (0xA7, "BTJZP", 22),
    (0x8E, "CALL", 43), (0x9E, "CALL", 45), (0xAE, "CALL", 12),
    (0xB5, "CLR A", 23), (0xC5, "CLR B", 23), (0xD5, "CLR", 24),
    (0xB0, "CLRC", 23),
    (0x6D, "CMP", 0), (0x1D, "CMP", 1), (0x3D, "CMP", 2), (0x4D, "CMP", 3),
    (0x2D, "CMP", 4), (0x5D, "CMP", 5), (0x7D, "CMP", 6),
    (0x8D, "CMPA", 10), (0x9D, "CMPA", 45), (0xAD, "CMPA", 12),
    (0x6E, "DAC", 0), (0x1E, "DAC", 1), (0x3E, "DAC", 2), (0x4E, "DAC", 3),
    (0x2E, "DAC", 4), (0x5E, "DAC", 5), (0x7E, "DAC", 6),
    (0xB2, "DEC A", 23), (0xC2, "DEC B", 23), (0xD2, "DEC", 24),
    (0xBB, "DECD A", 23), (0xCB, "DECD B", 23), (0xDB, "DECD", 24),
    (0x06, "DINT", 23),
    (0xBA, "DJNZ", 25), (0xCA, "DJNZ", 26), (0xDA, "DJNZ", 27),
    (0x6F, "DSB", 0), (0x1F, "DSB", 1), (0x3F, "DSB", 2), (0x4F, "DSB", 3),
    (0x2F, "DSB", 4), (0x5F, "DSB", 5), (0x7F, "DSB", 6),
    (0x05, "EINT", 23),
    (0x01, "IDLE", 23),
    (0xB3, "INC A", 23), (0xC3, "INC B", 23), (0xD3, "INC", 24),
    (0xB4, "INV A", 23), (0xC4, "INV B", 23), (0xD4, "INV", 24),
    (0xE2, "JEQ", 28), (0xE3, "JHS", 28), (0xE7, "JL", 28), (0xE0, "JMP", 28),
    (0xE1, "JN", 28), (0xE6, "JNZ", 28), (0xE4, "JP", 28), (0xE5, "JPZ", 28),
    (0x8A, "LDA", 10), (0x9A, "LDA", 45), (0xAA, "LDA", 12),
    (0x0D, "LDSP", 23),
    (0xC0, "MOV", 29), (0x62, "MOV", 30), (0xB1, "MOV", 30), (0xD0, "MOV", 31), (0xD1, "MOV", 32),
    (0x12, "MOV", 33), (0x32, "MOV", 34), (0x42, "MOV", 35), (0x22, "MOV", 36),
    (0x52, "MOV", 37), (0x72, "MOV", 38),
    (0x88, "MOVD", 39), (0x98, "MOVD", 35), (0xA8, "MOVD", 40),
    (0x82, "MOVP", 7), (0x92, "MOVP", 8), (0xA2, "MOVP", 9), (0x80, "MOVP", 41),
    (0x91, "MOVP", 42),
    (0x6C, "MPY", 0), (0x1C, "MPY", 1), (0x3C, "MPY", 2), (0x4C, "MPY", 3),
    (0x2C, "MPY", 4), (0x5C, "MPY", 5), (0x7C, "MPY", 6),
    (0x00, "NOP", 23),
    (0x64, "OR", 0), (0x14, "OR", 1), (0x34, "OR", 2), (0x44, "OR", 3),
    (0x24, "OR", 4), (0x54, "OR", 5), (0x74, "OR", 6),
    (0x84, "ORP", 7), (0x94, "ORP", 8), (0xA4, "ORP", 9),
    (0xB9, "POP A", 23), (0xC9, "POP B", 23), (0xD9, "POP", 24), (0x08, "POP ST", 23),
    (0xB8, "PUSH A", 23), (0xC8, "PUSH B", 23), (0xD8, "PUSH", 24), (0x0E, "PUSH ST", 23),
    (0x0B, "RETI", 23), (0x0A, "RETS", 23),
    (0xBE, "RL A", 23), (0xCE, "RL B", 23), (0xDE, "RL", 11),
    (0xBF, "RLC A", 23), (0xCF, "RLC B", 23), (0xDF, "RLC", 11),
    (0xBC, "RR A", 23), (0xCC, "RR B", 23), (0xDC, "RR", 11),
    (0xBD, "RRC A", 23), (0xCD, "RRC B", 23), (0xDD, "RRC", 11),
    (0x6B, "SBB", 0), (0x1B, "SBB", 1), (0x3B, "SBB", 2), (0x4B, "SBB", 3),
    (0x2B, "SBB", 4), (0x5B, "SBB", 5), (0x7B, "SBB", 6),
    (0x07, "SETC", 23),
    (0x8B, "STA", 10), (0x9B, "STA", 45), (0xAB, "STA", 12),
    (0x09, "STSP", 23),
    (0x6A, "SUB", 0), (0x1A, "SUB", 1), (0x3A, "SUB", 2), (0x4A, "SUB", 3),
    (0x2A, "SUB", 4), (0x5A, "SUB", 5), (0x7A, "SUB", 6),
    (0xFF, "TRAP 0", 44), (0xFE, "TRAP 1", 44), (0xFD, "TRAP 2", 44), (0xFC, "TRAP 3", 44),
    (0xFB, "TRAP 4", 44), (0xFA, "TRAP 5", 44), (0xF9, "TRAP 6", 44), (0xF8, "TRAP 7", 44),
    (0xF7, "TRAP 8", 44), (0xF6, "TRAP 9", 44), (0xF5, "TRAP 10", 44), (0xF4, "TRAP 11", 44),
    (0xF3, "TRAP 12", 44), (0xF2, "TRAP 13", 44), (0xF1, "TRAP 14", 44), (0xF0, "TRAP 15", 44),
    (0xEF, "TRAP 16", 44), (0xEE, "TRAP 17", 44), (0xED, "TRAP 18", 44), (0xEC, "TRAP 19", 44),
    (0xEB, "TRAP 20", 44), (0xEA, "TRAP 21", 44), (0xE9, "TRAP 22", 44), (0xE8, "TRAP 23", 44),
    (0xB7, "SWAP A", 23), (0xC7, "SWAP B", 23), (0xD7, "SWAP", 11),
    (0xB0, "TSTA", 23), (0xC1, "TSTB", 23),
    (0xB6, "XCHB A", 23), (0xC6, "XCHB B", 23), (0xD6, "XCHB", 11),
    (0x65, "XOR", 0), (0x15, "XOR", 1), (0x35, "XOR", 2), (0x45, "XOR", 3),
    (0x25, "XOR", 4), (0x55, "XOR", 5), (0x75, "XOR", 6),
    (0x85, "XORP", 7), (0x95, "XORP", 8), (0xA5, "XORP", 9),
];

/// Per-opcode `(mnemonic, format)`, first table entry wins.
pub(super) const LOOKUP: [Option<(&str, usize)>; 256] = {
    let mut table = [None; 256];
    let mut i = 0;
    while i < OPCODES.len() {
        let (op, name, fmt) = OPCODES[i];
        if table[op as usize].is_none() {
            table[op as usize] = Some((name, fmt));
        }
        i += 1;
    }
    table
};
