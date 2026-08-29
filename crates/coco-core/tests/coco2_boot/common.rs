use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
use test_assets::rom::{BAS12, EXTBAS11};

/// Extended Color BASIC occupies the low 8K of the flat ROM image ($8000-$9FFF).
pub const EXTBAS_LEN: usize = 8 * 1024;
/// Color BASIC occupies the high 8K ($A000-$BFFF) — `SAM_BAS_ROM_OFFSET` in
/// `bus.rs`, duplicated here as a test-local constant (that one is private).
pub const BAS_OFFSET: usize = 8 * 1024;

pub fn try_load(name: &str) -> Option<Vec<u8>> {
    let path = test_assets::rom(name);
    std::fs::read(&path).ok()
}

/// Compose the flat 16K ROM image the plain-SAM bus path expects: extbas at
/// offset 0, and Color BASIC at offset `BAS_OFFSET` (`bus.rs::
/// SAM_BAS_ROM_OFFSET`). Returns `None` if either file is missing, allowing the
/// test to skip.
fn load_coco2_rom() -> Option<Box<[u8]>> {
    let extbas = try_load(EXTBAS11)?;
    let bas = try_load(BAS12)?;
    assert_eq!(extbas.len(), EXTBAS_LEN, "extbas11.rom: unexpected size");
    assert_eq!(bas.len(), BAS_OFFSET, "bas12.rom: unexpected size");
    let mut image = extbas;
    image.extend_from_slice(&bas);
    Some(image.into_boxed_slice())
}

pub fn boot_machine() -> Option<(Machine, Vec<u8>)> {
    let extbas = try_load(EXTBAS11);
    let bas = try_load(BAS12);
    if extbas.is_none() || bas.is_none() {
        eprintln!(
            "skipping coco2_boot: extbas11.rom/bas12.rom not present in roms/ \
             (see the ROM files spec)"
        );
        return None;
    }
    let bas = bas.unwrap();
    let rom = load_coco2_rom().expect("checked Some above");
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    config
        .validate()
        .expect("Coco2/Ntsc/K64 must be a valid configuration");
    let m = Machine::new(config, rom);
    Some((m, bas))
}

pub fn screen_contains(m: &mut Machine, needle: &str) -> bool {
    m.text_screen_lines().iter().any(|l| l.contains(needle))
}

pub fn screen_dump(m: &mut Machine) -> String {
    m.text_screen_lines().join("\n")
}

pub fn tap_char(m: &mut Machine, c: char) {
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

pub fn type_line(m: &mut Machine, s: &str) {
    for c in s.chars().chain(std::iter::once('\r')) {
        tap_char(m, c);
    }
}

/// Run fields until the sign-on banner/`OK` prompt appears (real Color
/// BASIC's cold-start does a RAM-size probe, byte-by-byte across up to 64K,
/// before it can paint anything, hence the generous budget), then let the
/// housekeeping loop settle for a few more fields. Shared by every following test
/// that needs a machine sitting at the `OK` prompt, ready for direct-mode
/// input.
pub fn boot_to_prompt(m: &mut Machine) {
    const MAX_FIELDS: usize = 1500;
    let mut alive = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if screen_contains(m, "OK") && screen_contains(m, "EXTENDED COLOR BASIC 1.1") {
            alive = true;
            break;
        }
    }
    assert!(
        alive,
        "sign-on banner/OK prompt never appeared; screen:\n{}",
        screen_dump(m)
    );
    for _ in 0..10 {
        m.run_field();
    }
}

/// Field budget given to each direct-mode BASIC statement that follows to
/// tokenize/execute before the next one is typed (mirrors the settle time
/// `boot_to_prompt` already gives the cold-start banner).
pub const SETTLE_FIELDS: usize = 30;

pub fn run_fields(m: &mut Machine, n: usize) {
    for _ in 0..n {
        m.run_field();
    }
}

/// Field budget after typing `PRINT 2+2` for the answer to appear — shared
/// by every test that ends with this "still sane" check.
pub const ANSWER_FIELDS: usize = 120;

/// Type `PRINT 2+2` and confirm it evaluates to `4`, for tests that end with
/// a "still sane after some bus-level poking" check.
pub fn assert_print_2_plus_2_works(m: &mut Machine) {
    type_line(m, "PRINT 2+2");
    let mut answered = false;
    for _ in 0..ANSWER_FIELDS {
        m.run_field();
        if screen_contains(m, " 4") {
            answered = true;
            break;
        }
    }
    assert!(
        answered,
        "PRINT 2+2 never produced ' 4'; screen:\n{}",
        screen_dump(m)
    );
}
