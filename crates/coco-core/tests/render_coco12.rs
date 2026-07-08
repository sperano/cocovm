//! Deterministic coverage for the CoCo 1/2 fixed-VDG colour source
//! (`docs/coco12-plan.md` Phase 3): the legacy text/SG4/graphics renderers
//! must resolve to the exact MAME `mc6847.cpp` `s_palette` RGB values, not
//! the GIME palette registers (which don't exist on these machines). Style
//! mirrors `tests/render.rs`/`tests/render_graphics.rs`, but driven through
//! `Machine` (like `render.rs`'s `text_renderer_follows_sam_page_register`)
//! since the colour-source dispatch lives in `lib.rs`, not `video.rs` itself.

use coco_core::video::{BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, FB_W, VDG_FIXED_PALETTE};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, VideoStandard};
use mc6809::Bus;

/// PIA1 $FF22: A/G, GM2-0, CSS (SAM strobes move the display base, not this).
const FF22_AG: u8 = 0x80;
const FF22_CSS: u8 = 0x08;
/// PIA1 $FF22 GM2-0 = 111 (RG6 / PMODE 4).
const FF22_GM_RG6: u8 = 7 << 4;

/// Text/graphics base this suite uses throughout: SAM F1 set (`$FFC9`) moves
/// the display base to $0400 (2 * 512), keeping it clear of the parked `BRA *`
/// at $0000.
const SCREEN_BASE: u16 = 0x0400;

fn coco2_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::Ntsc,
        memory: MemorySize::K64,
    }
}

/// A CoCo 2 with a zeroed 16K synthetic ROM (so the reset vector, mirrored
/// from $BFFE/$BFFF, resolves to $0000) and the CPU parked on `BRA *` there,
/// so it never executes anything with side effects — matching
/// `tests/render.rs`'s `text_renderer_follows_sam_page_register`.
fn boot_parked_machine() -> Machine {
    let rom = vec![0u8; 16 * 1024].into_boxed_slice();
    let mut m = Machine::new(coco2_config(), rom);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.write(0xFFC9, 0); // SAM F1 set: display base -> $0400
    m
}

fn px(fb: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * FB_W + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

#[test]
fn text_uses_fixed_green_set_by_default() {
    let mut m = boot_parked_machine();
    m.bus.pia1.b.output = 0; // text mode (A/G clear), CSS=0
    m.bus.write(SCREEN_BASE, 0x00); // '@' glyph: many foreground strokes
    m.run_field();

    let fg = VDG_FIXED_PALETTE[13]; // ALPHANUMERIC BRIGHT GREEN
    let bg = VDG_FIXED_PALETTE[12]; // ALPHANUMERIC DARK GREEN
    let border = VDG_FIXED_PALETTE[8]; // BLACK

    assert_eq!(
        px(&m.framebuffer, 0, 0),
        border,
        "text border must be fixed black"
    );
    let mut fg_seen = false;
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            let p = px(&m.framebuffer, x, y);
            assert!(
                p == fg || p == bg,
                "unexpected colour {p:?} in glyph cell (not fg/bg green set)"
            );
            fg_seen |= p == fg;
        }
    }
    assert!(fg_seen, "'@' glyph produced no foreground pixels");
}

#[test]
fn text_uses_fixed_orange_set_when_css_set() {
    let mut m = boot_parked_machine();
    m.bus.pia1.b.output = FF22_CSS; // text mode, CSS=1
    m.bus.write(SCREEN_BASE, 0x00);
    m.run_field();

    let fg = VDG_FIXED_PALETTE[15]; // ALPHANUMERIC BRIGHT ORANGE
    let bg = VDG_FIXED_PALETTE[14]; // ALPHANUMERIC DARK ORANGE
    let mut fg_seen = false;
    for y in BORDER..BORDER + CELL_H {
        for x in BORDER..BORDER + CELL_W {
            let p = px(&m.framebuffer, x, y);
            assert!(
                p == fg || p == bg,
                "unexpected colour {p:?} in glyph cell (not fg/bg orange set)"
            );
            fg_seen |= p == fg;
        }
    }
    assert!(fg_seen, "'@' glyph produced no foreground pixels");
}

#[test]
fn semigraphics4_uses_fixed_vdg_colors() {
    let mut m = boot_parked_machine();
    m.bus.pia1.b.output = 0; // alphanumeric/SG4 mode
    // bit7=1 (SG4), colour = palette reg 3 (RED), pattern = upper-left + lower-right lit.
    const SG4: u8 = 0x80 | (3 << 4) | 0b1001;
    m.bus.write(SCREEN_BASE, SG4);
    m.run_field();

    let on = VDG_FIXED_PALETTE[3]; // RED
    let off = VDG_FIXED_PALETTE[8]; // BLACK
    let quad_x = CELL_W / 2;
    let quad_y = CELL_H / 2;
    assert_eq!(
        px(&m.framebuffer, BORDER, BORDER),
        on,
        "upper-left quadrant lit"
    );
    assert_eq!(
        px(&m.framebuffer, BORDER + quad_x, BORDER),
        off,
        "upper-right quadrant unlit"
    );
    assert_eq!(
        px(&m.framebuffer, BORDER, BORDER + quad_y),
        off,
        "lower-left quadrant unlit"
    );
    assert_eq!(
        px(&m.framebuffer, BORDER + quad_x, BORDER + quad_y),
        on,
        "lower-right quadrant lit"
    );
}

#[test]
fn pmode4_style_graphics_uses_fixed_colors_and_green_border() {
    let mut m = boot_parked_machine();
    m.bus.pia1.b.output = FF22_AG | FF22_GM_RG6; // RG6 / PMODE 4, CSS=0
    // SAM V0-V2 must pair with RG6 (V=111) for the vertical-cadence table
    // (`video::LEGACY_GFX_LINES_PER_ROW`).
    m.bus.write(0xFFC1, 0); // V0 set
    m.bus.write(0xFFC3, 0); // V1 set
    m.bus.write(0xFFC5, 0); // V2 set
    // First byte of the display: MSB (leftmost pixel) lit, the rest clear.
    m.bus.write(SCREEN_BASE, 0b1000_0000);
    m.run_field();

    let c0 = VDG_FIXED_PALETTE[8]; // palette reg 8 (2-colour "off", CSS=0): BLACK
    let c1 = VDG_FIXED_PALETTE[9]; // palette reg 9 (2-colour "on", CSS=0): GREEN
    let border = VDG_FIXED_PALETTE[0]; // graphics border, CSS=0: GREEN (MAME border_value)

    assert_eq!(
        px(&m.framebuffer, 0, 0),
        border,
        "graphics border must be green (CSS=0)"
    );
    assert_eq!(
        px(&m.framebuffer, BORDER, BORDER),
        c1,
        "MSB pixel = colour 1 (on)"
    );
    assert_eq!(
        px(&m.framebuffer, BORDER + 1, BORDER),
        c0,
        "next pixel = colour 0 (off)"
    );
}
