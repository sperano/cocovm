//! Phase 6 acceptance tests 1-3 (`docs/coco12-plan.md`): PMODE/SCREEN
//! graphics dispatch, the SAM R1 speed poke, and the SAM TY all-RAM flip —
//! each exercised through real ROM code typed at the `OK` prompt.

use mc6809::Bus;

use super::common::{
    SETTLE_FIELDS, assert_print_2_plus_2_works, boot_machine, boot_to_prompt, run_fields, type_line,
};

/// Phase 6 acceptance test 1 (`docs/coco12-plan.md`): `PMODE 4,1:SCREEN 1,1`
/// switches PIA1 $FF22's A/G bit on, `Machine::video_mode_summary` reports
/// the CoCo-compatible graphics dispatch, and the framebuffer's border and
/// interior pixels resolve through the fixed VDG palette
/// (`render_coco12.rs`'s unit-level coverage of the same colour source,
/// exercised here end-to-end through real ROM code).
///
/// `PMODE`/`SCREEN` must run from a *running program*, not typed directly at
/// the `OK` prompt: verified empirically against the real ROMs (traced via a
/// temporary `sam_write`/`sam_io_write` probe during development, since
/// nothing in `docs/coco12-plan.md` documents it) — Color BASIC's idle loop
/// (waiting for a keystroke at the prompt) re-asserts the SAM V0-V2/F0-F6
/// strobes and PIA1 $FF22 back to its text-mode defaults every field, so a
/// direct-mode `SCREEN 1,1` (or a raw `POKE 65314,...`) is visibly clobbered
/// again before the next field boundary. A one-line program that ends in an
/// infinite loop keeps the CPU out of that idle loop, so the mode sticks —
/// matching the well-known real-hardware behaviour that `PMODE`/`SCREEN`
/// only "hold" once `RUN`, not typed live.
#[test]
fn coco2_pmode_switches_to_graphics_with_fixed_vdg_colors() {
    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    type_line(&mut m, "10 PMODE 4,1:SCREEN 1,1:PCLS");
    run_fields(&mut m, SETTLE_FIELDS);
    type_line(&mut m, "20 GOTO 20");
    run_fields(&mut m, SETTLE_FIELDS);
    type_line(&mut m, "RUN");
    run_fields(&mut m, 2 * SETTLE_FIELDS);

    assert!(
        m.bus.pia1.b.output & coco_core::video::VDG_AG != 0,
        "PIA1 $FF22 A/G bit should be set after SCREEN 1,1; screen:\n{}",
        super::common::screen_dump(&mut m)
    );
    assert!(
        m.video_mode_summary().contains("PMODE"),
        "video_mode_summary should report CoCo-compatible graphics: {}",
        m.video_mode_summary()
    );

    let css = m.bus.pia1.b.output & coco_core::video::VDG_CSS != 0;
    let border_index = coco_core::video::vdg_graphics_border_index(css);
    let expected_border = coco_core::video::VDG_FIXED_PALETTE[border_index];

    let px = |fb: &[u8], x: usize, y: usize| -> [u8; 4] {
        let i = (y * coco_core::video::FB_W + x) * coco_core::video::BYTES_PER_PIXEL;
        fb[i..i + 4].try_into().unwrap()
    };
    assert_eq!(
        px(&m.framebuffer, 0, 0),
        expected_border,
        "graphics border should be the fixed VDG colour for CSS={css}"
    );

    // RG6/PMODE4's 2-colour table: palette regs 8/9 (CSS=0) or 10/11 (CSS=1)
    // — see `video.rs::vdg_palette_indices`/`render_coco12.rs`. Whatever
    // PCLS filled the page with, every interior pixel must resolve to one of
    // those two fixed colours, not e.g. a GIME-palette leftover.
    let (off_index, on_index) = if css { (10, 11) } else { (8, 9) };
    let off = coco_core::video::VDG_FIXED_PALETTE[off_index];
    let on = coco_core::video::VDG_FIXED_PALETTE[on_index];
    let interior = px(
        &m.framebuffer,
        coco_core::video::BORDER,
        coco_core::video::BORDER,
    );
    assert!(
        interior == off || interior == on,
        "interior graphics pixel should be one of the fixed RG6 2-colour VDG \
         colours for CSS={css}: got {interior:?}, expected {off:?} or {on:?}"
    );
}

/// Phase 6 acceptance test 2 (`docs/coco12-plan.md`): `POKE 65497,0` ($FFD9,
/// SAM R1 strobe) doubles the CPU rate, and `POKE 65496,0` ($FFD8) restores
/// it — matching the bus-level coverage in `tests/sam.rs`/`tests/speed.rs`,
/// exercised here through real ROM code typed at the `OK` prompt, with a
/// working `PRINT` afterward proving the machine is still sane post-flip.
#[test]
fn coco2_speed_poke_toggles_sam_r1_and_keeps_running() {
    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    assert!(!m.bus.sam.r1, "R1 should be clear at cold boot");

    type_line(&mut m, "POKE 65497,0");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(m.bus.sam.r1, "R1 should be set after POKE 65497,0 ($FFD9)");

    type_line(&mut m, "POKE 65496,0");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(
        !m.bus.sam.r1,
        "R1 should be clear again after POKE 65496,0 ($FFD8)"
    );

    assert_print_2_plus_2_works(&mut m);
}

/// Phase 6 acceptance test 3 (`docs/coco12-plan.md`): on a live-booted
/// machine, strobing SAM TY set (`$FFDF`) with M1 already set (64K) switches
/// $A000-$BFFF from ROM to RAM — matching `sam.rs`'s
/// `ty1_with_m1_maps_all_ram_through_feff_banking_out_rom` unit test, but
/// through the bus with a real ROM image underneath, and with a working
/// `PRINT` afterward proving BASIC survives the round trip. Bus-level (not
/// typed), per the plan: BASIC itself runs from ROM, so this can't be probed
/// purely from BASIC without an assembly stub.
#[test]
fn coco2_ffdf_all_ram_flip_on_live_boot() {
    let Some((mut m, bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

    const M1_SET: u16 = 0xFFDD;
    const TY_SET: u16 = 0xFFDF;
    const TY_CLEAR: u16 = 0xFFDE;
    const A000: u16 = 0xA000;

    let rom_byte = bas_rom[0]; // Color BASIC ROM's first byte, at $A000.
    assert_eq!(
        m.bus.read(A000),
        rom_byte,
        "before the flip, $A000 should read the real Color BASIC ROM byte"
    );

    m.bus.write(M1_SET, 0); // Force M1 (64K), regardless of BASIC's own sizing.
    m.bus.write(TY_SET, 0); // TY set: all-RAM, ROM disabled.

    const TEST_BYTE: u8 = 0xAB;
    m.bus.write(A000, TEST_BYTE);
    assert_eq!(
        m.bus.read(A000),
        TEST_BYTE,
        "with TY set, $A000 should be writable RAM, not read-only ROM"
    );

    m.bus.write(TY_CLEAR, 0); // Back to the ROM map.
    assert_eq!(
        m.bus.read(A000),
        rom_byte,
        "after clearing TY, $A000 should read the ROM byte again"
    );

    assert_print_2_plus_2_works(&mut m);
}
