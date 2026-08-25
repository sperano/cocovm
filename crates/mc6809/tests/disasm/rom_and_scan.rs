//! A known byte sequence from the real ROM, and a forward `len`-sum walk that
//! must never land mid-instruction.

use mc6809::disasm::disassemble;
use test_assets::rom::COCO3;

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
    let path = test_assets::rom(COCO3);
    let rom = std::fs::read(&path).expect("coco3.rom in the cocovm XDG data directory");
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
