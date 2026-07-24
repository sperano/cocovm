//! Test-driven coverage for the static disassembler (`mc6809::disasm`):
//! immediate/direct/extended/inherent addressing, short/long branches
//! (offset resolved to an absolute address), PSHS/PULS/PSHU/PULU register
//! masks, TFR/EXG register pairs, both prefix pages ($10/$11), illegal
//! opcodes in all three pages, a known byte sequence from the real ROM, and
//! a forward `len`-sum walk that must never land mid-instruction.
//!
//! Indexed-addressing postbyte coverage lives in `disasm_indexed.rs`.

use mc6809::disasm::disassemble;

/// Disassemble `bytes` (padded with zeros) as if loaded at `addr`.
fn disasm_at(addr: u16, bytes: &[u8]) -> mc6809::disasm::Insn {
    let mem = bytes.to_vec();
    disassemble(
        &mut |a: u16| {
            let idx = a.wrapping_sub(addr) as usize;
            mem.get(idx).copied().unwrap_or(0)
        },
        addr,
    )
}

// ---- Inherent -------------------------------------------------------------

#[test]
fn inherent_nop() {
    let insn = disasm_at(0x1000, &[0x12]);
    assert_eq!(insn.mnemonic, "NOP");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);
}

#[test]
fn inherent_rts_abx_rti_mul_sex_daa_sync_swi() {
    for (byte, mnem) in [
        (0x13u8, "SYNC"),
        (0x19, "DAA"),
        (0x1D, "SEX"),
        (0x39, "RTS"),
        (0x3A, "ABX"),
        (0x3B, "RTI"),
        (0x3D, "MUL"),
        (0x3F, "SWI"),
    ] {
        let insn = disasm_at(0x2000, &[byte]);
        assert_eq!(insn.mnemonic, mnem, "opcode ${byte:02X}");
        assert_eq!(insn.operand, "");
        assert_eq!(insn.len, 1);
    }
}

// ---- Immediate --------------------------------------------------------

#[test]
fn immediate_8bit_lda() {
    let insn = disasm_at(0x1000, &[0x86, 0x3F]);
    assert_eq!(insn.mnemonic, "LDA");
    assert_eq!(insn.operand, "#$3F");
    assert_eq!(insn.len, 2);
}

#[test]
fn immediate_16bit_ldd() {
    let insn = disasm_at(0x1000, &[0xCC, 0x12, 0x34]);
    assert_eq!(insn.mnemonic, "LDD");
    assert_eq!(insn.operand, "#$1234");
    assert_eq!(insn.len, 3);
}

#[test]
fn orcc_andcc_cwai_are_immediate8() {
    for (byte, mnem) in [(0x1Au8, "ORCC"), (0x1C, "ANDCC"), (0x3C, "CWAI")] {
        let insn = disasm_at(0x1000, &[byte, 0x50]);
        assert_eq!(insn.mnemonic, mnem);
        assert_eq!(insn.operand, "#$50");
        assert_eq!(insn.len, 2);
    }
}

// ---- Direct / Extended --------------------------------------------------

#[test]
fn direct_sta() {
    let insn = disasm_at(0x1000, &[0x97, 0x80]);
    assert_eq!(insn.mnemonic, "STA");
    assert_eq!(insn.operand, "$80");
    assert_eq!(insn.len, 2);
}

#[test]
fn extended_jsr() {
    let insn = disasm_at(0x1000, &[0xBD, 0xA9, 0x28]);
    assert_eq!(insn.mnemonic, "JSR");
    assert_eq!(insn.operand, "$A928");
    assert_eq!(insn.len, 3);
}

#[test]
fn extended_clr_from_rmw_group() {
    // 0x7F: extended-mode RMW, low nibble 0xF = CLR.
    let insn = disasm_at(0x1000, &[0x7F, 0xFF, 0x90]);
    assert_eq!(insn.mnemonic, "CLR");
    assert_eq!(insn.operand, "$FF90");
    assert_eq!(insn.len, 3);
}

#[test]
fn inherent_rmw_on_accumulators() {
    // 0x43 = COMA (0x40-0x4F, nibble 3 = COM); 0x5B = illegal nibble (0xB) on B.
    let insn = disasm_at(0x1000, &[0x43]);
    assert_eq!(insn.mnemonic, "COMA");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 1);

    let insn = disasm_at(0x1000, &[0x5B]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$5B");
    assert_eq!(insn.len, 1);
}

#[test]
fn direct_rmw_illegal_nibble_still_consumes_direct_byte() {
    // 0x01: direct-mode RMW, nibble 1 is illegal — but `step()` calls
    // `ea_direct` unconditionally before checking the nibble, so the direct
    // byte IS consumed (len stays 2, matching the addressing mode).
    let insn = disasm_at(0x1000, &[0x01, 0x42]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$42");
    assert_eq!(insn.len, 2);
}

// ---- LEA ------------------------------------------------------------------

#[test]
fn leax_indexed() {
    let insn = disasm_at(0x1000, &[0x30, 0x05]); // LEAX 5,X
    assert_eq!(insn.mnemonic, "LEAX");
    assert_eq!(insn.operand, "5,X");
    assert_eq!(insn.len, 2);
}

// ---- Branches: short -----------------------------------------------------

#[test]
fn short_branch_offset_resolved_to_absolute_forward() {
    // BRA +4 at $1000: opcode@1000, offset@1001 -> next-pc 0x1002; target 0x1006.
    let insn = disasm_at(0x1000, &[0x20, 0x04]);
    assert_eq!(insn.mnemonic, "BRA");
    assert_eq!(insn.operand, "$1006");
    assert_eq!(insn.len, 2);
}

#[test]
fn short_branch_offset_resolved_to_absolute_backward() {
    // BEQ -2 -> self-loop: next-pc 0x1002, target 0x1000.
    let insn = disasm_at(0x1000, &[0x27, 0xFE]);
    assert_eq!(insn.mnemonic, "BEQ");
    assert_eq!(insn.operand, "$1000");
    assert_eq!(insn.len, 2);
}

#[test]
fn all_short_branch_mnemonics() {
    let expected = [
        "BRA", "BRN", "BHI", "BLS", "BCC", "BCS", "BNE", "BEQ", "BVC", "BVS", "BPL", "BMI", "BGE",
        "BLT", "BGT", "BLE",
    ];
    for (nibble, mnem) in expected.iter().enumerate() {
        let opcode = 0x20 + nibble as u8;
        let insn = disasm_at(0x1000, &[opcode, 0x00]);
        assert_eq!(insn.mnemonic, *mnem, "opcode ${opcode:02X}");
        assert_eq!(insn.len, 2);
    }
}

#[test]
fn bsr_is_relative_like_short_branch() {
    let insn = disasm_at(0x1000, &[0x8D, 0x10]); // BSR +16 -> target 0x1012
    assert_eq!(insn.mnemonic, "BSR");
    assert_eq!(insn.operand, "$1012");
    assert_eq!(insn.len, 2);
}

// ---- Branches: long --------------------------------------------------

#[test]
fn lbra_long_branch() {
    let insn = disasm_at(0x1000, &[0x16, 0x01, 0x00]); // LBRA +0x100 -> target 0x1103
    assert_eq!(insn.mnemonic, "LBRA");
    assert_eq!(insn.operand, "$1103");
    assert_eq!(insn.len, 3);
}

#[test]
fn lbsr_long_branch() {
    let insn = disasm_at(0x1000, &[0x17, 0x00, 0x10]);
    assert_eq!(insn.mnemonic, "LBSR");
    assert_eq!(insn.operand, "$1013");
    assert_eq!(insn.len, 3);
}

// ---- PSH/PUL ------------------------------------------------------------

#[test]
fn pshs_all_registers_low_to_high_order() {
    let insn = disasm_at(0x1000, &[0x34, 0xFF]);
    assert_eq!(insn.mnemonic, "PSHS");
    assert_eq!(insn.operand, "CC,A,B,DP,X,Y,U,PC");
    assert_eq!(insn.len, 2);
}

#[test]
fn puls_other_stack_ptr_bit_names_u() {
    // mask 0x40 alone: PULS names the "other stack pointer" bit U.
    let insn = disasm_at(0x1000, &[0x35, 0x40]);
    assert_eq!(insn.mnemonic, "PULS");
    assert_eq!(insn.operand, "U");
    assert_eq!(insn.len, 2);
}

#[test]
fn pshu_other_stack_ptr_bit_names_s() {
    let insn = disasm_at(0x1000, &[0x36, 0x40]);
    assert_eq!(insn.mnemonic, "PSHU");
    assert_eq!(insn.operand, "S");
    assert_eq!(insn.len, 2);
}

#[test]
fn pulu_partial_mask() {
    // mask 0x16 = A(0x02)+B(0x04)+X(0x10)
    let insn = disasm_at(0x1000, &[0x37, 0x16]);
    assert_eq!(insn.mnemonic, "PULU");
    assert_eq!(insn.operand, "A,B,X");
    assert_eq!(insn.len, 2);
}

// ---- TFR/EXG --------------------------------------------------------------

#[test]
fn tfr_16bit_pair() {
    let insn = disasm_at(0x1000, &[0x1F, 0x12]); // TFR X,Y (src=1,dst=2)
    assert_eq!(insn.mnemonic, "TFR");
    assert_eq!(insn.operand, "X,Y");
    assert_eq!(insn.len, 2);
}

#[test]
fn exg_8bit_pair() {
    let insn = disasm_at(0x1000, &[0x1E, 0x89]); // EXG A,B (src=8,dst=9)
    assert_eq!(insn.mnemonic, "EXG");
    assert_eq!(insn.operand, "A,B");
    assert_eq!(insn.len, 2);
}

#[test]
fn tfr_invalid_register_code_marked_clearly() {
    let insn = disasm_at(0x1000, &[0x1F, 0x06]); // src=0 (D), dst=6 (invalid)
    assert_eq!(insn.mnemonic, "TFR");
    assert_eq!(insn.operand, "D,?6");
    assert_eq!(insn.len, 2);
}

// ---- Prefix pages -----------------------------------------------------

#[test]
fn page10_long_branch() {
    let insn = disasm_at(0x1000, &[0x10, 0x26, 0x00, 0x05]); // LBNE +5 -> target 0x1009
    assert_eq!(insn.mnemonic, "LBNE");
    assert_eq!(insn.operand, "$1009");
    assert_eq!(insn.len, 4);
}

#[test]
fn page10_ldy_immediate_and_sty_extended() {
    let insn = disasm_at(0x1000, &[0x10, 0x8E, 0x20, 0x00]);
    assert_eq!(insn.mnemonic, "LDY");
    assert_eq!(insn.operand, "#$2000");
    assert_eq!(insn.len, 4);

    let insn = disasm_at(0x1000, &[0x10, 0xBF, 0x30, 0x00]);
    assert_eq!(insn.mnemonic, "STY");
    assert_eq!(insn.operand, "$3000");
    assert_eq!(insn.len, 4);
}

#[test]
fn page10_swi2() {
    let insn = disasm_at(0x1000, &[0x10, 0x3F]);
    assert_eq!(insn.mnemonic, "SWI2");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 2);
}

#[test]
fn page10_illegal_second_byte_is_length_2() {
    // 0x10 0x00: not decoded by the core (falls to its 2-cycle default after
    // only the prefix + second byte are read).
    let insn = disasm_at(0x1000, &[0x10, 0x00]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$00");
    assert_eq!(insn.len, 2);
}

#[test]
fn page11_cmpu_and_cmps() {
    let insn = disasm_at(0x1000, &[0x11, 0x83, 0x00, 0x10]);
    assert_eq!(insn.mnemonic, "CMPU");
    assert_eq!(insn.operand, "#$0010");
    assert_eq!(insn.len, 4);

    let insn = disasm_at(0x1000, &[0x11, 0xAC, 0x84]); // CMPS ,X
    assert_eq!(insn.mnemonic, "CMPS");
    assert_eq!(insn.operand, ",X");
    assert_eq!(insn.len, 3);
}

#[test]
fn page11_swi3() {
    let insn = disasm_at(0x1000, &[0x11, 0x3F]);
    assert_eq!(insn.mnemonic, "SWI3");
    assert_eq!(insn.operand, "");
    assert_eq!(insn.len, 2);
}

#[test]
fn page11_illegal_second_byte_is_length_2() {
    let insn = disasm_at(0x1000, &[0x11, 0xFF]);
    assert_eq!(insn.mnemonic, "???");
    assert_eq!(insn.operand, "$FF");
    assert_eq!(insn.len, 2);
}

// ---- Illegal base-page opcodes --------------------------------------------

#[test]
fn illegal_base_page_opcodes_are_length_1() {
    // The documented illegal set (0x14, 0x15, 0x18, 0x1B, 0x38, 0x3E) plus the
    // "STx immediate" slots that don't exist on real hardware: 0x87 (STA),
    // 0x8F (STX), 0xC7 (STB), 0xCD (STD), 0xCF (STU) — see the crate-level
    // task report for the two of these five a prior spec pass missed.
    for byte in [
        0x14u8, 0x15, 0x18, 0x1B, 0x38, 0x3E, 0x87, 0x8F, 0xC7, 0xCD, 0xCF,
    ] {
        let insn = disasm_at(0x1000, &[byte]);
        assert_eq!(insn.mnemonic, "???", "opcode ${byte:02X}");
        assert_eq!(insn.operand, format!("${byte:02X}"));
        assert_eq!(insn.len, 1, "opcode ${byte:02X}");
    }
}

// ---- Known ROM byte sequence -----------------------------------------

/// Hand-decoded from `roms/coco3.rom` starting at the RESET entry point.
/// Reset vector at file offset 0x7FFE-0x7FFF ($FFFE-$FFFF) reads `8C 1B` ->
/// entry point $8C1B. File offset for $8C1B = 0x8C1B - 0x8000 = 0x0C1B.
/// Bytes at that offset (verified via `xxd -s 0xc1b -l 48 roms/coco3.rom`):
///
/// ```text
/// 1a50 860a b7ff 907f ffde 7ec0 007f feed
/// 7fff 2386 ccb7 ff90 7fff de39 3416 9e88
/// d6e7 1026 6b6d e661 7ea3 0e34 010d e727
/// ```
///
/// Hand-decoded instruction stream (addr: bytes -> mnemonic operand):
/// ```text
/// 8C1B: 1A 50        ORCC #$50
/// 8C1D: 86 0A        LDA  #$0A
/// 8C1F: B7 FF 90     STA  $FF90
/// 8C22: 7F FF DE     CLR  $FFDE
/// 8C25: 7E C0 00     JMP  $C000
/// 8C28: 7F FE ED     CLR  $FEED
/// 8C2B: 7F FF 23     CLR  $FF23
/// 8C2E: 86 CC        LDA  #$CC
/// 8C30: B7 FF 90     STA  $FF90
/// 8C33: 7F FF DE     CLR  $FFDE
/// 8C36: 39           RTS
/// 8C37: 34 16        PSHS A,B,X        (mask 0x16 = A|B|X)
/// 8C39: 9E 88        LDX  $88
/// 8C3B: D6 E7        LDB  $E7
/// 8C3D: 10 26 6B 6D  LBNE $F7AE        ($10 page, nibble 6 = BNE -> LBNE)
/// 8C41: E6 61        LDB  1,S          (postbyte 0x61: bit7 clear, rr=11=S, n=1)
/// ```
#[test]
fn rom_reset_entry_point_disassembles_as_hand_decoded() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    let rom = std::fs::read(&path).expect("roms/coco3.rom (git-ignored, local-only)");
    // ROM maps to $8000-$FFFF; file offset = addr - $8000.
    let reset_vector = ((rom[0x7FFE] as u16) << 8) | rom[0x7FFF] as u16;
    assert_eq!(reset_vector, 0x8C1B, "reset vector changed — ROM mismatch");

    let read = |addr: u16| -> u8 {
        let offset = addr.wrapping_sub(0x8000) as usize;
        rom[offset]
    };

    let expected: &[(u16, &str, &str, u8)] = &[
        (0x8C1B, "ORCC", "#$50", 2),
        (0x8C1D, "LDA", "#$0A", 2),
        (0x8C1F, "STA", "$FF90", 3),
        (0x8C22, "CLR", "$FFDE", 3),
        (0x8C25, "JMP", "$C000", 3),
        (0x8C28, "CLR", "$FEED", 3),
        (0x8C2B, "CLR", "$FF23", 3),
        (0x8C2E, "LDA", "#$CC", 2),
        (0x8C30, "STA", "$FF90", 3),
        (0x8C33, "CLR", "$FFDE", 3),
        (0x8C36, "RTS", "", 1),
        (0x8C37, "PSHS", "A,B,X", 2),
        (0x8C39, "LDX", "$88", 2),
        (0x8C3B, "LDB", "$E7", 2),
        (0x8C3D, "LBNE", "$F7AE", 4),
        (0x8C41, "LDB", "1,S", 2),
    ];

    let mut pc = 0x8C1Bu16;
    let mut read_fn = read;
    for &(addr, mnemonic, operand, len) in expected {
        assert_eq!(pc, addr, "pc drifted before decoding {mnemonic}");
        let insn = disassemble(&mut read_fn, pc);
        assert_eq!(insn.mnemonic, mnemonic, "at ${pc:04X}");
        assert_eq!(insn.operand, operand, "at ${pc:04X}");
        assert_eq!(insn.len, len, "at ${pc:04X}");
        pc = pc.wrapping_add(insn.len as u16);
    }
}

// ---- len-sum forward scan never desyncs -----------------------------------

#[test]
fn forward_scan_never_lands_mid_instruction() {
    // Straight-line hand-assembled sequence mixing several lengths (1..3
    // bytes): NOP(1), LDA #imm(2), STA direct(2), JMP extended(3), RTS(1).
    // Each PC advance must land exactly on the next opcode we expect.
    let addr = 0x4000u16;
    let bytes: &[u8] = &[
        0x12, // NOP                 @4000 len1
        0x86, 0x7E, // LDA #$7E      @4001 len2
        0x97, 0x50, // STA $50       @4003 len2
        0x7E, 0x50, 0x00, // JMP $5000 @4005 len3
        0x39, // RTS                 @4008 len1
    ];
    let mem = bytes.to_vec();
    let mut read_fn = move |a: u16| {
        let idx = a.wrapping_sub(addr) as usize;
        mem.get(idx).copied().unwrap_or(0)
    };

    let expected_starts = [0x4000u16, 0x4001, 0x4003, 0x4005, 0x4008];
    let expected_mnemonics = ["NOP", "LDA", "STA", "JMP", "RTS"];

    let mut pc = addr;
    for (i, &start) in expected_starts.iter().enumerate() {
        assert_eq!(pc, start, "instruction {i} start address drifted");
        let insn = disassemble(&mut read_fn, pc);
        assert_eq!(insn.mnemonic, expected_mnemonics[i], "instruction {i}");
        pc = pc.wrapping_add(insn.len as u16);
    }
    assert_eq!(pc, 0x4009); // one past RTS, end of the buffer
}
