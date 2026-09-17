//! Deterministic coverage for the CoCo 1/2 fixed-VDG colour source: the
//! legacy text, SG4, and graphics renderers must resolve to the exact MAME
//! `mc6847.cpp` `s_palette` RGB values, not
//! the GIME palette registers, which don't exist on these machines. The tests
//! follow `tests/render.rs` and `tests/render_graphics.rs`, but run through
//! `Machine` (like `render.rs`'s `text_renderer_follows_sam_page_register`)
//! since the colour-source dispatch lives in `lib.rs`, not `video.rs` itself.

use coco_core::video::{CELL_H, CELL_W, VDG_FIXED_PALETTE};
use mc6809::Bus;

use super::common::{SCREEN_BASE, boot_parked_machine, dot, px};

/// PIA1 $FF22: A/G, GM2-0, CSS (SAM strobes move the display base, not this).
const FF22_AG: u8 = 0x80;
const FF22_CSS: u8 = 0x08;
/// PIA1 $FF22 GM2-0 = 111 (RG6 / PMODE 4).
const FF22_GM_RG6: u8 = 7 << 4;
const SAM_V1_SET: u16 = 0xFFC3;
const SAM_V2_SET: u16 = 0xFFC5;

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
    for y in 0..CELL_H {
        for x in 0..CELL_W {
            let p = dot(&m.framebuffer, x, y);
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
    for y in 0..CELL_H {
        for x in 0..CELL_W {
            let p = dot(&m.framebuffer, x, y);
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
    assert_eq!(dot(&m.framebuffer, 0, 0), on, "upper-left quadrant lit");
    assert_eq!(
        dot(&m.framebuffer, quad_x, 0),
        off,
        "upper-right quadrant unlit"
    );
    assert_eq!(
        dot(&m.framebuffer, 0, quad_y),
        off,
        "lower-left quadrant unlit"
    );
    assert_eq!(
        dot(&m.framebuffer, quad_x, quad_y),
        on,
        "lower-right quadrant lit"
    );
}

#[test]
fn pmode4_style_graphics_uses_fixed_colors_and_green_border() {
    let mut m = boot_parked_machine();
    m.bus.pia1.b.output = FF22_AG | FF22_GM_RG6; // RG6 / PMODE 4, CSS=0
    // RG6's stock SAM pairing is V=110; V=111 is the MC6883 DMA mode.
    m.bus.write(SAM_V1_SET, 0);
    m.bus.write(SAM_V2_SET, 0);
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
    assert_eq!(dot(&m.framebuffer, 0, 0), c1, "MSB pixel = colour 1 (on)");
    assert_eq!(dot(&m.framebuffer, 1, 0), c0, "next pixel = colour 0 (off)");
}
