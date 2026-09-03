//! Microsequencer operand-block layouts: how each LOAD/DELTA/SETMSB opcode's
//! bit-packed operands map onto the 16 filter registers, per mode (MAME
//! `sp0256.cpp` `sp0256_datafmt`/`sp0256_df_idx`, Joe Zbiciak's
//! reverse-engineering). Transcribed verbatim; keep the row numbering.

use super::lpc::reg::{
    AMPLITUDE as AM, AMPLITUDE_INTERP as IA, B0, B1, B2, B3, B4, B5, F0, F1, F2, F3, F4, F5,
    PERIOD as PR, PERIOD_INTERP as IP,
};

/// One control word of an operand block: which register a field updates and
/// how the fetched bits are decoded.
#[derive(Clone, Copy)]
pub(super) struct Field {
    /// Bits to fetch (0 = fetch nothing; the entry only clears registers).
    pub len: u8,
    /// Left shift applied to the fetched value. For a field replace, also
    /// how many low register bits survive.
    pub shift: u8,
    /// Target register index ([`super::lpc::reg`]).
    pub param: usize,
    /// Delta update: sign-extend the field and add it to the register.
    pub delta: bool,
    /// Field replace: merge the shifted field over the register's high bits.
    pub field: bool,
    /// Clear B5/F5 first.
    pub clear5: bool,
    /// Clear all 16 registers first.
    pub clear_all: bool,
}

/// MAME's `CR(len, shift, param, delta, field, clr5, clrall)` macro.
const fn cr(
    len: u8,
    shift: u8,
    param: usize,
    delta: u8,
    field: u8,
    clear5: u8,
    clear_all: u8,
) -> Field {
    Field {
        len,
        shift,
        param,
        delta: delta != 0,
        field: field != 0,
        clear5: clear5 != 0,
        clear_all: clear_all != 0,
    }
}

/// Placeholder for MAME's two unused rows (43, 44).
const UNUSED: Field = cr(0, 0, 0, 0, 0, 0, 0);

/// The operand-block control words, indexed by [`DF_IDX`] ranges.
#[rustfmt::skip]
pub(super) const DATAFMT: [Field; 177] = [
    // OPCODE 1111: PAUSE
    /*   0 */ cr(0, 0, 0, 0, 0, 0, 1),
    // Opcode 0001: LOADALL — all modes
    /*   1 */ cr(8, 0, AM, 0, 0, 0, 1),
    /*   2 */ cr(8, 0, PR, 0, 0, 0, 0),
    /*   3 */ cr(8, 0, B0, 0, 0, 0, 0),
    /*   4 */ cr(8, 0, F0, 0, 0, 0, 0),
    /*   5 */ cr(8, 0, B1, 0, 0, 0, 0),
    /*   6 */ cr(8, 0, F1, 0, 0, 0, 0),
    /*   7 */ cr(8, 0, B2, 0, 0, 0, 0),
    /*   8 */ cr(8, 0, F2, 0, 0, 0, 0),
    /*   9 */ cr(8, 0, B3, 0, 0, 0, 0),
    /*  10 */ cr(8, 0, F3, 0, 0, 0, 0),
    /*  11 */ cr(8, 0, B4, 0, 0, 0, 0),
    /*  12 */ cr(8, 0, F4, 0, 0, 0, 0),
    /*  13 */ cr(8, 0, B5, 0, 0, 0, 0),
    /*  14 */ cr(8, 0, F5, 0, 0, 0, 0),
    // Mode 01 and 11 only
    /*  15 */ cr(8, 0, IA, 0, 0, 0, 0),
    /*  16 */ cr(8, 0, IP, 0, 0, 0, 0),
    // Opcode 0100: LOAD_4 — mode 00 and 01
    /*  17 */ cr(6, 2, AM, 0, 0, 0, 1),
    /*  18 */ cr(8, 0, PR, 0, 0, 0, 0),
    /*  19 */ cr(4, 3, B3, 0, 0, 0, 0),
    /*  20 */ cr(6, 2, F3, 0, 0, 0, 0),
    /*  21 */ cr(7, 1, B4, 0, 0, 0, 0),
    /*  22 */ cr(6, 2, F4, 0, 0, 0, 0),
    // Mode 01 only
    /*  23 */ cr(8, 0, B5, 0, 0, 0, 0),
    /*  24 */ cr(8, 0, F5, 0, 0, 0, 0),
    // Mode 10 and 11
    /*  25 */ cr(6, 2, AM, 0, 0, 0, 1),
    /*  26 */ cr(8, 0, PR, 0, 0, 0, 0),
    /*  27 */ cr(6, 1, B3, 0, 0, 0, 0),
    /*  28 */ cr(7, 1, F3, 0, 0, 0, 0),
    /*  29 */ cr(8, 0, B4, 0, 0, 0, 0),
    /*  30 */ cr(8, 0, F4, 0, 0, 0, 0),
    // Mode 11 only
    /*  31 */ cr(8, 0, B5, 0, 0, 0, 0),
    /*  32 */ cr(8, 0, F5, 0, 0, 0, 0),
    // Opcode 0110: SETMSB_6 — mode 00 only
    /*  33 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 00 and 01
    /*  34 */ cr(6, 2, AM, 0, 0, 0, 0),
    /*  35 */ cr(6, 2, F3, 0, 1, 0, 0),
    /*  36 */ cr(6, 2, F4, 0, 1, 0, 0),
    // Mode 01 only
    /*  37 */ cr(8, 0, F5, 0, 1, 0, 0),
    // Mode 10 only
    /*  38 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 10 and 11
    /*  39 */ cr(6, 2, AM, 0, 0, 0, 0),
    /*  40 */ cr(7, 1, F3, 0, 1, 0, 0),
    /*  41 */ cr(8, 0, F4, 0, 1, 0, 0),
    // Mode 11 only
    /*  42 */ cr(8, 0, F5, 0, 1, 0, 0),
    /*  43 */ UNUSED,
    /*  44 */ UNUSED,
    // Opcode 1001: DELTA_9 — mode 00 and 01
    /*  45 */ cr(4, 2, AM, 1, 0, 0, 0),
    /*  46 */ cr(5, 0, PR, 1, 0, 0, 0),
    /*  47 */ cr(3, 4, B0, 1, 0, 0, 0),
    /*  48 */ cr(3, 3, F0, 1, 0, 0, 0),
    /*  49 */ cr(3, 4, B1, 1, 0, 0, 0),
    /*  50 */ cr(3, 3, F1, 1, 0, 0, 0),
    /*  51 */ cr(3, 4, B2, 1, 0, 0, 0),
    /*  52 */ cr(3, 3, F2, 1, 0, 0, 0),
    /*  53 */ cr(3, 3, B3, 1, 0, 0, 0),
    /*  54 */ cr(4, 2, F3, 1, 0, 0, 0),
    /*  55 */ cr(4, 1, B4, 1, 0, 0, 0),
    /*  56 */ cr(4, 2, F4, 1, 0, 0, 0),
    // Mode 01 only
    /*  57 */ cr(5, 0, B5, 1, 0, 0, 0),
    /*  58 */ cr(5, 0, F5, 1, 0, 0, 0),
    // Mode 10 and 11
    /*  59 */ cr(4, 2, AM, 1, 0, 0, 0),
    /*  60 */ cr(5, 0, PR, 1, 0, 0, 0),
    /*  61 */ cr(4, 1, B0, 1, 0, 0, 0),
    /*  62 */ cr(4, 2, F0, 1, 0, 0, 0),
    /*  63 */ cr(4, 1, B1, 1, 0, 0, 0),
    /*  64 */ cr(4, 2, F1, 1, 0, 0, 0),
    /*  65 */ cr(4, 1, B2, 1, 0, 0, 0),
    /*  66 */ cr(4, 2, F2, 1, 0, 0, 0),
    /*  67 */ cr(4, 1, B3, 1, 0, 0, 0),
    /*  68 */ cr(5, 1, F3, 1, 0, 0, 0),
    /*  69 */ cr(5, 0, B4, 1, 0, 0, 0),
    /*  70 */ cr(5, 0, F4, 1, 0, 0, 0),
    // Mode 11 only
    /*  71 */ cr(5, 0, B5, 1, 0, 0, 0),
    /*  72 */ cr(5, 0, F5, 1, 0, 0, 0),
    // Opcode 1010: SETMSB_A — mode 00 only
    /*  73 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 00 and 01
    /*  74 */ cr(6, 2, AM, 0, 0, 0, 0),
    /*  75 */ cr(5, 3, F0, 0, 1, 0, 0),
    /*  76 */ cr(5, 3, F1, 0, 1, 0, 0),
    /*  77 */ cr(5, 3, F2, 0, 1, 0, 0),
    // Mode 10 only
    /*  78 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 10 and 11
    /*  79 */ cr(6, 2, AM, 0, 0, 0, 0),
    /*  80 */ cr(6, 2, F0, 0, 1, 0, 0),
    /*  81 */ cr(6, 2, F1, 0, 1, 0, 0),
    /*  82 */ cr(6, 2, F2, 0, 1, 0, 0),
    // Opcode 0010: LOAD_2 / 1100: LOAD_C — mode 00
    /*  83 */ cr(6, 2, AM, 0, 0, 0, 1),
    /*  84 */ cr(8, 0, PR, 0, 0, 0, 0),
    /*  85 */ cr(3, 4, B0, 0, 0, 0, 0),
    /*  86 */ cr(5, 3, F0, 0, 0, 0, 0),
    /*  87 */ cr(3, 4, B1, 0, 0, 0, 0),
    /*  88 */ cr(5, 3, F1, 0, 0, 0, 0),
    /*  89 */ cr(3, 4, B2, 0, 0, 0, 0),
    /*  90 */ cr(5, 3, F2, 0, 0, 0, 0),
    /*  91 */ cr(4, 3, B3, 0, 0, 0, 0),
    /*  92 */ cr(6, 2, F3, 0, 0, 0, 0),
    /*  93 */ cr(7, 1, B4, 0, 0, 0, 0),
    /*  94 */ cr(6, 2, F4, 0, 0, 0, 0),
    // LOAD_2 only
    /*  95 */ cr(5, 0, IA, 0, 0, 0, 0),
    /*  96 */ cr(5, 0, IP, 0, 0, 0, 0),
    // LOAD_2 / LOAD_C — mode 10
    /*  97 */ cr(6, 2, AM, 0, 0, 0, 1),
    /*  98 */ cr(8, 0, PR, 0, 0, 0, 0),
    /*  99 */ cr(6, 1, B0, 0, 0, 0, 0),
    /* 100 */ cr(6, 2, F0, 0, 0, 0, 0),
    /* 101 */ cr(6, 1, B1, 0, 0, 0, 0),
    /* 102 */ cr(6, 2, F1, 0, 0, 0, 0),
    /* 103 */ cr(6, 1, B2, 0, 0, 0, 0),
    /* 104 */ cr(6, 2, F2, 0, 0, 0, 0),
    /* 105 */ cr(6, 1, B3, 0, 0, 0, 0),
    /* 106 */ cr(7, 1, F3, 0, 0, 0, 0),
    /* 107 */ cr(8, 0, B4, 0, 0, 0, 0),
    /* 108 */ cr(8, 0, F4, 0, 0, 0, 0),
    // LOAD_2 only
    /* 109 */ cr(5, 0, IA, 0, 0, 0, 0),
    /* 110 */ cr(5, 0, IP, 0, 0, 0, 0),
    // OPCODE 1101: DELTA_D — mode 00 and 01
    /* 111 */ cr(4, 2, AM, 1, 0, 0, 0),
    /* 112 */ cr(5, 0, PR, 1, 0, 0, 0),
    /* 113 */ cr(3, 3, B3, 1, 0, 0, 0),
    /* 114 */ cr(4, 2, F3, 1, 0, 0, 0),
    /* 115 */ cr(4, 1, B4, 1, 0, 0, 0),
    /* 116 */ cr(4, 2, F4, 1, 0, 0, 0),
    // Mode 01 only
    /* 117 */ cr(5, 0, B5, 1, 0, 0, 0),
    /* 118 */ cr(5, 0, F5, 1, 0, 0, 0),
    // Mode 10 and 11
    /* 119 */ cr(4, 2, AM, 1, 0, 0, 0),
    /* 120 */ cr(5, 0, PR, 1, 0, 0, 0),
    /* 121 */ cr(4, 1, B3, 1, 0, 0, 0),
    /* 122 */ cr(5, 1, F3, 1, 0, 0, 0),
    /* 123 */ cr(5, 0, B4, 1, 0, 0, 0),
    /* 124 */ cr(5, 0, F4, 1, 0, 0, 0),
    // Mode 11 only
    /* 125 */ cr(5, 0, B5, 1, 0, 0, 0),
    /* 126 */ cr(5, 0, F5, 1, 0, 0, 0),
    // OPCODE 1110: LOAD_E
    /* 127 */ cr(6, 2, AM, 0, 0, 0, 0),
    /* 128 */ cr(8, 0, PR, 0, 0, 0, 0),
    // LOAD_2 / LOAD_C — mode 01
    /* 129 */ cr(6, 2, AM, 0, 0, 0, 1),
    /* 130 */ cr(8, 0, PR, 0, 0, 0, 0),
    /* 131 */ cr(3, 4, B0, 0, 0, 0, 0),
    /* 132 */ cr(5, 3, F0, 0, 0, 0, 0),
    /* 133 */ cr(3, 4, B1, 0, 0, 0, 0),
    /* 134 */ cr(5, 3, F1, 0, 0, 0, 0),
    /* 135 */ cr(3, 4, B2, 0, 0, 0, 0),
    /* 136 */ cr(5, 3, F2, 0, 0, 0, 0),
    /* 137 */ cr(4, 3, B3, 0, 0, 0, 0),
    /* 138 */ cr(6, 2, F3, 0, 0, 0, 0),
    /* 139 */ cr(7, 1, B4, 0, 0, 0, 0),
    /* 140 */ cr(6, 2, F4, 0, 0, 0, 0),
    /* 141 */ cr(8, 0, B5, 0, 0, 0, 0),
    /* 142 */ cr(8, 0, F5, 0, 0, 0, 0),
    // LOAD_2 only
    /* 143 */ cr(5, 0, IA, 0, 0, 0, 0),
    /* 144 */ cr(5, 0, IP, 0, 0, 0, 0),
    // LOAD_2 / LOAD_C — mode 11
    /* 145 */ cr(6, 2, AM, 0, 0, 0, 1),
    /* 146 */ cr(8, 0, PR, 0, 0, 0, 0),
    /* 147 */ cr(6, 1, B0, 0, 0, 0, 0),
    /* 148 */ cr(6, 2, F0, 0, 0, 0, 0),
    /* 149 */ cr(6, 1, B1, 0, 0, 0, 0),
    /* 150 */ cr(6, 2, F1, 0, 0, 0, 0),
    /* 151 */ cr(6, 1, B2, 0, 0, 0, 0),
    /* 152 */ cr(6, 2, F2, 0, 0, 0, 0),
    /* 153 */ cr(6, 1, B3, 0, 0, 0, 0),
    /* 154 */ cr(7, 1, F3, 0, 0, 0, 0),
    /* 155 */ cr(8, 0, B4, 0, 0, 0, 0),
    /* 156 */ cr(8, 0, F4, 0, 0, 0, 0),
    /* 157 */ cr(8, 0, B5, 0, 0, 0, 0),
    /* 158 */ cr(8, 0, F5, 0, 0, 0, 0),
    // LOAD_2 only
    /* 159 */ cr(5, 0, IA, 0, 0, 0, 0),
    /* 160 */ cr(5, 0, IP, 0, 0, 0, 0),
    // Opcode 0011: SETMSB_3 / 0101: SETMSB_5 — mode 00 only
    /* 161 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 00 and 01
    /* 162 */ cr(6, 2, AM, 0, 0, 0, 0),
    /* 163 */ cr(8, 0, PR, 0, 0, 0, 0),
    /* 164 */ cr(5, 3, F0, 0, 1, 0, 0),
    /* 165 */ cr(5, 3, F1, 0, 1, 0, 0),
    /* 166 */ cr(5, 3, F2, 0, 1, 0, 0),
    // SETMSB_3 only
    /* 167 */ cr(5, 0, IA, 0, 0, 0, 0),
    /* 168 */ cr(5, 0, IP, 0, 0, 0, 0),
    // Mode 10 only
    /* 169 */ cr(0, 0, 0, 0, 0, 1, 0),
    // Mode 10 and 11
    /* 170 */ cr(6, 2, AM, 0, 0, 0, 0),
    /* 171 */ cr(8, 0, PR, 0, 0, 0, 0),
    /* 172 */ cr(6, 2, F0, 0, 1, 0, 0),
    /* 173 */ cr(6, 2, F1, 0, 1, 0, 0),
    /* 174 */ cr(6, 2, F2, 0, 1, 0, 0),
    // SETMSB_3 only
    /* 175 */ cr(5, 0, IA, 0, 0, 0, 0),
    /* 176 */ cr(5, 0, IP, 0, 0, 0, 0),
];

/// `(first, last)` [`DATAFMT`] row per `(opcode, mode)`, flattened as
/// `opcode * 8 + (mode & 6)`. Rows are numeric opcodes 0-15 as fetched
/// (MAME's row comments spell them bit-reversed); `-1` marks opcodes that
/// never carry an operand block.
#[rustfmt::skip]
const DF_IDX: [i16; 128] = [
    /* opcode 0x0 RTS/SETPAGE */ -1, -1, -1, -1, -1, -1, -1, -1,
    /* opcode 0x1 SETMODE     */ -1, -1, -1, -1, -1, -1, -1, -1,
    /* opcode 0x2 LOAD_4      */ 17, 22, 17, 24, 25, 30, 25, 32,
    /* opcode 0x3 LOAD_C      */ 83, 94, 129, 142, 97, 108, 145, 158,
    /* opcode 0x4 LOAD_2      */ 83, 96, 129, 144, 97, 110, 145, 160,
    /* opcode 0x5 SETMSB_A    */ 73, 77, 74, 77, 78, 82, 79, 82,
    /* opcode 0x6 SETMSB_6    */ 33, 36, 34, 37, 38, 41, 39, 42,
    /* opcode 0x7 LOAD_E      */ 127, 128, 127, 128, 127, 128, 127, 128,
    /* opcode 0x8 LOADALL     */ 1, 14, 1, 16, 1, 14, 1, 16,
    /* opcode 0x9 DELTA_9     */ 45, 56, 45, 58, 59, 70, 59, 72,
    /* opcode 0xA SETMSB_5    */ 161, 166, 162, 166, 169, 174, 170, 174,
    /* opcode 0xB DELTA_D     */ 111, 116, 111, 118, 119, 124, 119, 126,
    /* opcode 0xC SETMSB_3    */ 161, 168, 162, 168, 169, 176, 170, 176,
    /* opcode 0xD JSR         */ -1, -1, -1, -1, -1, -1, -1, -1,
    /* opcode 0xE JMP         */ -1, -1, -1, -1, -1, -1, -1, -1,
    /* opcode 0xF PAUSE       */ 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The inclusive [`DATAFMT`] row range for `opcode` (4-bit, as fetched) in
/// `mode` (only bits 1-2 select), or `None` for opcodes without operands.
pub(super) fn block_range(opcode: u8, mode: u8) -> Option<(usize, usize)> {
    let i = usize::from(opcode & 0xF) * 8 + usize::from(mode & 6);
    let first = usize::try_from(DF_IDX[i]).ok()?;
    let last = usize::try_from(DF_IDX[i + 1]).ok()?;
    Some((first, last))
}
