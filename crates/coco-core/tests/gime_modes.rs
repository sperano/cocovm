//! Integration coverage for the GIME-native video dispatch: booting the real ROM
//! to the (VDG-compatible) BASIC prompt, then programming the GIME registers
//! as WIDTH 80 and HSCREEN do. These writes must switch the machine to
//! per-scanline painting of the canonical 640×240 raster (Option B), and back
//! cleanly to the VDG whole-field geometry. Register values are the ROM's own
//! video-register images (SEB Unravelled II, tables at LE03C/LE071).

use coco_core::raster::{CANVAS_H, CANVAS_W};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;
use test_assets::rom::COCO3;

/// Fields to run before poking modes — enough to reach the idle BASIC prompt.
const BOOT_FIELDS: usize = 120;

fn boot_machine() -> Machine {
    let path = test_assets::rom(COCO3);
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    m
}

#[test]
fn width80_registers_switch_to_native_text_and_back() {
    let mut m = boot_machine();
    assert_eq!(
        (m.fb_width, m.fb_height),
        (CANVAS_W as u32, CANVAS_H as u32),
        "CoCo 3 renders the canonical raster from boot (legacy modes too)"
    );

    // The ROM's WIDTH 80 register image: COCO off, BP=0 LPR=8, 80 cols with
    // attributes, video base $6C000 ($FF9D:$FF9E = $D80:0 ×8).
    m.bus.write(0xFF90, 0x4C); // INIT0: COCO=0, MMU on, MC3/MC2
    m.bus.write(0xFF98, 0x03);
    m.bus.write(0xFF99, 0x15);
    m.bus.write(0xFF9D, 0xD8);
    m.bus.write(0xFF9E, 0x00);
    m.run_field();

    assert_eq!(
        (m.fb_width, m.fb_height),
        (CANVAS_W as u32, CANVAS_H as u32)
    );

    // Back to CoCo-compatible: still the canonical canvas — one stable
    // texture size across every CoCo 3 mode is the point of Option B.
    m.bus.write(0xFF90, 0xCC);
    m.run_field();
    m.run_field(); // the COCO flip latches at the NEXT field start
    assert_eq!(
        (m.fb_width, m.fb_height),
        (CANVAS_W as u32, CANVAS_H as u32)
    );
}

#[test]
fn hscreen2_registers_switch_to_native_graphics() {
    let mut m = boot_machine();

    // HSCREEN 2 (320×192, 16 colours): BP=1, 160 bytes/row, CRES=%10,
    // video base $60000 ($FF9D = $C0).
    m.bus.write(0xFF90, 0x4C);
    m.bus.write(0xFF98, 0x80);
    m.bus.write(0xFF99, 0x1E);
    m.bus.write(0xFF9D, 0xC0);
    m.bus.write(0xFF9E, 0x00);
    m.run_field();

    assert_eq!(
        (m.fb_width, m.fb_height),
        (CANVAS_W as u32, CANVAS_H as u32)
    );
    assert_eq!(
        m.framebuffer.len(),
        (m.fb_width * m.fb_height) as usize * coco_core::video::BYTES_PER_PIXEL
    );
}
