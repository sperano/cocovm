//! Integration coverage for `SystemBus` + the real Super Extended Color BASIC
//! ROM: the CPU must fetch the reset vector from ROM, execute the cold-start code
//! out of internal ROM, and reach the I/O page — build-order step 2 (`DESIGN.md`
//! §10). This is the first "run real code" milestone; a per-instruction trace
//! diff against XRoar/MAME (step 3) will catch subtler bugs later.

use std::path::PathBuf;

use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

/// Cold-start entry point in the diskless CoCo 3 ROM (reset vector `$FFFE`).
const RESET_ENTRY: u16 = 0x8C1B;

fn load_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

#[test]
fn reset_vector_points_into_rom() {
    let mut m = boot_machine();
    // $FFFE/$FFFF are fetched from internal ROM even though they live in the I/O page.
    assert_eq!(m.bus.read(0xFFFE), 0x8C);
    assert_eq!(m.bus.read(0xFFFF), 0x1B);
    // Machine::new already ran reset(), so PC should be the cold-start entry.
    assert_eq!(m.cpu.pc, RESET_ENTRY);
}

#[test]
fn rom_window_reads_internal_rom() {
    let mut m = boot_machine();
    // $8000 == ROM offset 0 == "EX" signature of Extended Color BASIC.
    assert_eq!(m.bus.read(0x8000), b'E');
    assert_eq!(m.bus.read(0x8001), b'X');
}

#[test]
fn cold_start_configures_rom_and_jumps_into_upper_half() {
    // The first five instructions are deterministic and exercise the whole slice:
    // ORCC (mask), LDA #$0A / STA $FF90 (INIT0), CLR $FF91 (INIT1), JMP $C000.
    const ROM_CONFIG_STEPS: usize = 5;
    // INIT0 written by the cold-start: MC1 set (32K internal ROM), MC3 set,
    // MMU still disabled.
    const EXPECTED_INIT0: u8 = 0x0A;
    const UPPER_ROM_ENTRY: u16 = 0xC000;

    let mut m = boot_machine();
    for _ in 0..ROM_CONFIG_STEPS {
        m.step();
    }

    // The GIME latched the INIT0/INIT1 writes (proves the I/O decode is live).
    assert_eq!(m.bus.gime.init0, EXPECTED_INIT0);
    assert!(!m.bus.gime.mmu_enabled);
    assert_eq!(m.bus.gime.init1, 0);
    // The JMP landed in the (now internal) upper 16K of ROM.
    assert_eq!(m.cpu.pc, UPPER_ROM_ENTRY);
    // And that address really reads internal ROM (offset $4000), not open bus.
    assert_ne!(m.bus.read(UPPER_ROM_ENTRY), 0xFF);
}

#[test]
fn boot_initializes_palette_via_io_writes() {
    // A little further in, the cold-start clears the 16 palette registers with a
    // `STA ,X+` loop over $FFB0–$FFBF. Running enough steps must leave those GIME
    // palette registers written and never panic on any bus access.
    const STEPS: usize = 5_000;
    let mut m = boot_machine();
    for _ in 0..STEPS {
        m.step(); // any out-of-range bus access would panic here
    }
    // The palette loop writes A=$12 across the bank; at minimum the registers are
    // no longer all-zero, confirming palette-range I/O writes reached the GIME.
    assert!(
        m.bus.gime.palette.iter().any(|&p| p != 0),
        "palette registers were never written during boot"
    );
}
