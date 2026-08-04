//! Phase 6 (first slice,): a CoCo 2 running real
//! Extended Color BASIC 1.1 + Color BASIC 1.2 ROMs boots to the sign-on
//! banner and evaluates `PRINT 2+2` — the milestone that proves the SAM
//! primary memory map (Phase 2), the VDG-native colour/mode dispatch (Phase
//! 3), and the per-variant field loop (Phase 4) all work together on real
//! ROM code, not just unit tests.
//!
//! Skipped (not failed) if the ROMs aren't present locally, matching
//! `tests/boot.rs`/`tests/alive.rs`.

use mc6809::Bus;

use super::common::{assert_print_2_plus_2_works, boot_machine, boot_to_prompt};

/// Offset of the 6809 hardware vectors within an 8K Color BASIC ROM (the top
/// 32 bytes, $BFE0-$BFFF relative to the ROM's own $A000 base).
const BAS_VECTOR_OFFSET: usize = 0x1FE0;

#[test]
fn coco2_boots_extended_color_basic_and_evaluates_print() {
    let Some((mut m, bas_rom)) = boot_machine() else {
        return;
    };

    // The 6809 hardware vectors always read through the SAM's $FFE0-$FFFF ->
    // $BFE0-$BFFF mirror onto Color BASIC's own ROM, regardless of the SAM's
    // TY/M1 state. Check
    // the reset vector explicitly: it must match the last two bytes of the
    // real bas12.rom dump, not just "some" value.
    let expected_reset_hi = bas_rom[BAS_VECTOR_OFFSET + 0x1E]; // $BFFE
    let expected_reset_lo = bas_rom[BAS_VECTOR_OFFSET + 0x1F]; // $BFFF
    assert_eq!(
        m.bus.read(0xFFFE),
        expected_reset_hi,
        "reset vector high byte via $FFFE mirror"
    );
    assert_eq!(
        m.bus.read(0xFFFF),
        expected_reset_lo,
        "reset vector low byte via $FFFF mirror"
    );
    assert_eq!(
        m.cpu.pc,
        u16::from_be_bytes([expected_reset_hi, expected_reset_lo]),
        "Machine::new's reset() must have fetched PC from that same vector"
    );

    boot_to_prompt(&mut m);
    assert_print_2_plus_2_works(&mut m);
}
