//! Integration coverage for the GIME-native video dispatch: booting the real ROM
//! to the (VDG-compatible) BASIC prompt, then programming the GIME registers the
//! way WIDTH 80 / HSCREEN do must switch `Machine::render_field` to the native
//! renderers, resize the framebuffer to the mode's native geometry, and switch
//! back cleanly. Register values are the ROM's own video-register images
//! (SEB Unravelled II, tables at LE03C/LE071).

use std::path::PathBuf;

use coco_core::gime_video::{BORDER_X_DIVISOR, BORDER_Y};
use coco_core::video;
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

/// Fields to run before poking modes — enough to reach the idle BASIC prompt.
const BOOT_FIELDS: usize = 120;

fn boot_machine() -> Machine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    m
}

fn fb_dims(active_w: usize, active_h: usize) -> (u32, u32) {
    (
        (active_w + 2 * (active_w / BORDER_X_DIVISOR)) as u32,
        (active_h + 2 * BORDER_Y) as u32,
    )
}

#[test]
fn width80_registers_switch_to_native_text_and_back() {
    let mut m = boot_machine();
    assert_eq!(
        (m.fb_width, m.fb_height),
        (video::FB_W as u32, video::FB_H as u32),
        "boots in the VDG-compatible geometry"
    );

    // The ROM's WIDTH 80 register image: COCO off, BP=0 LPR=8, 80 cols with
    // attributes, video base $6C000 ($FF9D:$FF9E = $D80:0 ×8).
    m.bus.write(0xFF90, 0x4C); // INIT0: COCO=0, MMU on, MC3/MC2
    m.bus.write(0xFF98, 0x03);
    m.bus.write(0xFF99, 0x15);
    m.bus.write(0xFF9D, 0xD8);
    m.bus.write(0xFF9E, 0x00);
    m.run_field();

    assert_eq!((m.fb_width, m.fb_height), fb_dims(80 * 8, 192));

    // Back to CoCo-compatible: the framebuffer returns to the fixed VDG geometry.
    m.bus.write(0xFF90, 0xCC);
    m.run_field();
    assert_eq!(
        (m.fb_width, m.fb_height),
        (video::FB_W as u32, video::FB_H as u32)
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

    assert_eq!((m.fb_width, m.fb_height), fb_dims(320, 192));
    assert_eq!(
        m.framebuffer.len(),
        (m.fb_width * m.fb_height) as usize * video::BYTES_PER_PIXEL
    );
}
