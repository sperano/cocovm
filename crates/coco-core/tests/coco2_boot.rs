//! Phase 6 (first slice, `docs/coco12-plan.md`): a CoCo 2 running real
//! Extended Color BASIC 1.1 + Color BASIC 1.2 ROMs boots to the sign-on
//! banner and evaluates `PRINT 2+2` — the milestone that proves the SAM
//! primary memory map (Phase 2), the VDG-native colour/mode dispatch (Phase
//! 3), and the per-variant field loop (Phase 4) all work together on real
//! ROM code, not just unit tests.
//!
//! Skipped (not failed) if the ROMs aren't present locally, matching
//! `tests/boot.rs`/`tests/alive.rs`.

use std::path::PathBuf;

use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, VideoStandard};
use mc6809::Bus;

/// Extended Color BASIC occupies the low 8K of the flat ROM image ($8000-$9FFF).
const EXTBAS_LEN: usize = 8 * 1024;
/// Color BASIC occupies the high 8K ($A000-$BFFF) — `SAM_BAS_ROM_OFFSET` in
/// `bus.rs`, duplicated here as a test-local constant (that one is private).
const BAS_OFFSET: usize = 8 * 1024;
/// Offset of the 6809 hardware vectors within an 8K Color BASIC ROM (the top
/// 32 bytes, $BFE0-$BFFF relative to the ROM's own $A000 base).
const BAS_VECTOR_OFFSET: usize = 0x1FE0;

fn try_load(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok()
}

/// Compose the flat 16K ROM image the plain-SAM bus path expects: extbas at
/// offset 0, bas at offset `BAS_OFFSET` (`docs/coco12-plan.md` "ROM files";
/// `bus.rs::SAM_BAS_ROM_OFFSET`). Returns `None` (test should skip) if either
/// file is missing.
fn load_coco2_rom() -> Option<Box<[u8]>> {
    let extbas = try_load("extbas11.rom")?;
    let bas = try_load("bas12.rom")?;
    assert_eq!(extbas.len(), EXTBAS_LEN, "extbas11.rom: unexpected size");
    assert_eq!(bas.len(), BAS_OFFSET, "bas12.rom: unexpected size");
    let mut image = extbas;
    image.extend_from_slice(&bas);
    Some(image.into_boxed_slice())
}

fn boot_machine() -> Option<(Machine, Vec<u8>)> {
    let extbas = try_load("extbas11.rom");
    let bas = try_load("bas12.rom");
    if extbas.is_none() || bas.is_none() {
        eprintln!(
            "skipping coco2_boot: extbas11.rom/bas12.rom not present in roms/ \
             (see docs/coco12-plan.md \"ROM files\")"
        );
        return None;
    }
    let bas = bas.unwrap();
    let rom = load_coco2_rom().expect("checked Some above");
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::Ntsc,
        memory: MemorySize::K64,
    };
    config
        .validate()
        .expect("Coco2/Ntsc/K64 must be a valid configuration");
    let m = Machine::new(config, rom);
    Some((m, bas))
}

fn screen_contains(m: &mut Machine, needle: &str) -> bool {
    m.text_screen_lines().iter().any(|l| l.contains(needle))
}

fn screen_dump(m: &mut Machine) -> String {
    m.text_screen_lines().join("\n")
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn type_line(m: &mut Machine, s: &str) {
    for c in s.chars().chain(std::iter::once('\r')) {
        tap_char(m, c);
    }
}

#[test]
fn coco2_boots_extended_color_basic_and_evaluates_print() {
    let Some((mut m, bas_rom)) = boot_machine() else {
        return;
    };

    // The 6809 hardware vectors always read through the SAM's $FFE0-$FFFF ->
    // $BFE0-$BFFF mirror onto Color BASIC's own ROM, regardless of the SAM's
    // TY/M1 state (`docs/coco12-plan.md`; `sam.rs::VECTOR_MIRROR_BASE`). Check
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

    // Generous field budget: real Color BASIC's cold-start does a RAM-size
    // probe (byte-by-byte across up to 64K) before it can paint anything.
    const MAX_FIELDS: usize = 1500;
    let mut alive = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if screen_contains(&mut m, "OK") && screen_contains(&mut m, "EXTENDED COLOR BASIC 1.1") {
            alive = true;
            break;
        }
    }
    assert!(
        alive,
        "sign-on banner/OK prompt never appeared; screen:\n{}",
        screen_dump(&mut m)
    );

    for _ in 0..10 {
        m.run_field();
    }
    type_line(&mut m, "PRINT 2+2");

    const ANSWER_FIELDS: usize = 120;
    let mut answered = false;
    for _ in 0..ANSWER_FIELDS {
        m.run_field();
        if screen_contains(&mut m, " 4") {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "PRINT 2+2 never produced ' 4'; screen:\n{}",
        screen_dump(&mut m)
    );
}
