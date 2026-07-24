//! Phase 6 acceptance test 5 (`docs/coco12-plan.md`): a CoCo 1 running real
//! Color BASIC 1.2 alone (no Extended Color BASIC) boots to the plain
//! "COLOR BASIC 1.2" sign-on banner — not "EXTENDED COLOR BASIC" — and
//! evaluates `PRINT 2+2`. This exercises the open-bus extbas window
//! (`docs/coco12-plan.md` "ROM files": "Extended Color BASIC missing → still
//! boot (Color BASIC only, open-bus filler $FF for the extbas half"), which
//! `coco2_boot.rs`'s extbas+bas machine never touches.
//!
//! Skipped (not failed) if `roms/bas12.rom` isn't present locally, matching
//! `tests/coco2_boot.rs`.

use std::path::PathBuf;

use coco_core::{
    Machine, MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
use mc6809::Bus;

/// Color BASIC occupies the high 8K ($A000-$BFFF) of the flat image, same
/// offset the plain-SAM bus expects extbas+bas machines to use
/// (`bus.rs::SAM_BAS_ROM_OFFSET`) — here the low 8K is left at
/// [`OPEN_BUS_FILLER`] instead of real Extended BASIC ROM contents.
const BAS_OFFSET: usize = 8 * 1024;
/// Open-bus fill byte for the (absent) extbas half, per
/// `docs/coco12-plan.md` "ROM files".
const OPEN_BUS_FILLER: u8 = 0xFF;

fn try_load(name: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok()
}

/// Compose a Color-BASIC-only flat image: `OPEN_BUS_FILLER` for the extbas
/// half, `bas12.rom` at `BAS_OFFSET` — exactly what `coco-egui`'s
/// `compose_coco12_rom` builds when no Extended BASIC dump is found.
fn boot_machine() -> Option<(Machine, Vec<u8>)> {
    let Some(bas) = try_load("bas12.rom") else {
        eprintln!(
            "skipping coco1_boot: bas12.rom not present in roms/ (see docs/coco12-plan.md \"ROM files\")"
        );
        return None;
    };
    let mut image = vec![OPEN_BUS_FILLER; BAS_OFFSET];
    image.extend_from_slice(&bas);
    let config = MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K32,
        monitor: MonitorType::RGB,
        vdg: VDGVariant::MC6847,
    };
    config
        .validate()
        .expect("Coco1/Ntsc/K32 must be a valid configuration");
    let m = Machine::new(config, image.into_boxed_slice());
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
fn coco1_boots_color_basic_only_and_evaluates_print() {
    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };

    // Sanity check the open-bus extbas window before booting: every byte
    // through the $8000-$9FFF flat-image half must read the conventional
    // open-bus filler, per `docs/coco12-plan.md`.
    for addr in 0x8000..0x9FFFu16 {
        assert_eq!(
            m.bus.read(addr),
            OPEN_BUS_FILLER,
            "extbas window at ${addr:04X} should be open bus without an Extended BASIC ROM"
        );
    }

    const MAX_FIELDS: usize = 1500;
    let mut alive = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        if screen_contains(&mut m, "OK") && screen_contains(&mut m, "COLOR BASIC 1.2") {
            alive = true;
            break;
        }
    }
    assert!(
        alive,
        "sign-on banner/OK prompt never appeared; screen:\n{}",
        screen_dump(&mut m)
    );
    assert!(
        !screen_contains(&mut m, "EXTENDED"),
        "a Color-BASIC-only machine must not show the Extended BASIC banner; screen:\n{}",
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
