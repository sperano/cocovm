//! Deterministic coverage for the CoCo 1/2 fixed-VDG colour source
//! (`docs/coco12-plan.md` Phase 3): the legacy text/SG4/graphics renderers
//! must resolve to the exact MAME `mc6847.cpp` `s_palette` RGB values, not
//! the GIME palette registers (which don't exist on these machines). Style
//! mirrors `tests/render.rs`/`tests/render_graphics.rs`, but driven through
//! `Machine` (like `render.rs`'s `text_renderer_follows_sam_page_register`)
//! since the colour-source dispatch lives in `lib.rs`, not `video.rs` itself.

use coco_core::gime::init0;
use coco_core::video::{
    BORDER, BYTES_PER_PIXEL, CELL_H, CELL_W, FB_W, TEXT_FG_INDEX, VDG_FIXED_PALETTE,
    VDG_GM0_INTEXT,
};
use coco_core::{
    Machine, MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
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
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: MonitorType::RGB,
        vdg: VDGVariant::MC6847,
    }
}

/// A CoCo 1 (forced plain MC6847 by [`MachineConfig::validate`]).
fn coco1_config() -> MachineConfig {
    MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K32,
        monitor: MonitorType::RGB,
        vdg: VDGVariant::MC6847,
    }
}

/// A CoCo 2 with the MC6847T1 installed.
fn coco2_t1_config() -> MachineConfig {
    MachineConfig {
        vdg: VDGVariant::MC6847T1,
        ..coco2_config()
    }
}

/// A machine with a zeroed 16K synthetic ROM (so the reset vector, mirrored
/// from $BFFE/$BFFF, resolves to $0000) and the CPU parked on `BRA *` there,
/// so it never executes anything with side effects — matching
/// `tests/render.rs`'s `text_renderer_follows_sam_page_register`.
fn boot_parked_machine_with(config: MachineConfig) -> Machine {
    let rom = vec![0u8; 16 * 1024].into_boxed_slice();
    let mut m = Machine::new(config, rom);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.write(0xFFC9, 0); // SAM F1 set: display base -> $0400
    m
}

fn boot_parked_machine() -> Machine {
    boot_parked_machine_with(coco2_config())
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

// --- MC6847 vs MC6847T1 font/lowercase (`crates/coco-core/src/font6847.rs`) ---
//
// Expected glyph bit patterns below are copied from (and cross-checked
// against the unit tests alongside) `src/font6847.rs`'s `MC6847_FONT`/
// `MC6847T1_FONT` tables: 'O' (code $0F) at plain-font index 15 / T1-font
// index 15, and lowercase 'a' at T1-font index 64+1=65 (`font6847.rs`'s
// module doc: index 64-95 = lowercase, entry 65 = 'a', screen code $01).

/// VDG screen code for 'O' (`@`=$00, so 'O' = $0F).
const CODE_O: u8 = 0x0F;
/// VDG screen code for 'A' (`@`=$00, so 'A' = $01).
const CODE_A: u8 = 0x01;
/// MC6847 alphanumeric INVERSE_BIT (bit 6).
const INVERSE_BIT: u8 = 0x40;

/// `MC6847_FONT[15]` ('O'): square shape (`font6847.rs::tests::plain_o_is_square`).
const PLAIN_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x3E, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3E, 0x00, 0x00,
];
/// `MC6847T1_FONT[15]` ('O'): rounded shape (`font6847.rs::tests::t1_o_is_rounded`).
const T1_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x1C, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1C, 0x00, 0x00, 0x00, 0x00,
];
/// `MC6847T1_FONT[64 + 1]` (true-lowercase 'a').
const T1_LOWER_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x1C, 0x02, 0x1E, 0x22, 0x1E, 0x00, 0x00, 0x00, 0x00,
];
/// `MC6847_FONT[1]` ('A'): plain chip, rows 3-9.
const PLAIN_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00,
];
/// `MC6847T1_FONT[1]` ('A'): T1 chip, rows 1-7 (2 rows higher than the plain
/// chip — see `src/font6847.rs`'s module doc on the per-chip row offset).
const T1_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x08, 0x14, 0x22, 0x22, 0x3E, 0x22, 0x22, 0x00, 0x00, 0x00, 0x00,
];

/// Sample the 8×12 cell at (row, col) into a bit grid: `true` where the
/// pixel equals `on_color`, `false` where it equals `off_color` (panics on
/// any other colour — every glyph pixel must be one or the other).
fn sample_cell(
    fb: &[u8],
    row: usize,
    col: usize,
    on_color: [u8; 4],
    off_color: [u8; 4],
) -> [[bool; CELL_W]; CELL_H] {
    let mut out = [[false; CELL_W]; CELL_H];
    for (cy, row_out) in out.iter_mut().enumerate() {
        for (cx, bit) in row_out.iter_mut().enumerate() {
            let p = px(fb, BORDER + col * CELL_W + cx, BORDER + row * CELL_H + cy);
            *bit = if p == on_color {
                true
            } else if p == off_color {
                false
            } else {
                panic!("unexpected colour {p:?} at cell ({row},{col}) px ({cx},{cy})");
            };
        }
    }
    out
}

/// Decode a raw font row byte array into the same bit-grid shape
/// [`sample_cell`] produces (leftmost pixel = bit mask `0x80 >> col`,
/// matching `video.rs::blit_cell`).
fn glyph_bits(glyph: &[u8; CELL_H]) -> [[bool; CELL_W]; CELL_H] {
    let mut out = [[false; CELL_W]; CELL_H];
    for (cy, &bits) in glyph.iter().enumerate() {
        for (cx, bit) in out[cy].iter_mut().enumerate() {
            *bit = bits & (0x80 >> cx) != 0;
        }
    }
    out
}

#[test]
fn plain_vdg_draws_square_o_t1_draws_rounded_o() {
    let mut plain = boot_parked_machine_with(coco1_config());
    plain.bus.pia1.b.output = 0; // text mode, CSS=0
    plain.bus.write(SCREEN_BASE, CODE_O);
    plain.run_field();

    let mut t1 = boot_parked_machine_with(coco2_t1_config());
    t1.bus.pia1.b.output = 0; // text mode, CSS=0, GM0 clear (no lowercase; code >= $0F irrelevant anyway)
    t1.bus.write(SCREEN_BASE, CODE_O);
    t1.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];

    let plain_cell = sample_cell(&plain.framebuffer, 0, 0, fg, bg);
    let t1_cell = sample_cell(&t1.framebuffer, 0, 0, fg, bg);

    assert_eq!(
        plain_cell,
        glyph_bits(&PLAIN_O_GLYPH),
        "plain MC6847 must draw the square 'O'"
    );
    assert_eq!(
        t1_cell,
        glyph_bits(&T1_O_GLYPH),
        "MC6847T1 must draw the rounded 'O'"
    );
    assert_ne!(
        plain_cell, t1_cell,
        "plain and T1 'O' glyphs must differ in shape"
    );
}

#[test]
fn t1_true_lowercase_requires_gm0_and_swaps_colors() {
    let mut m = boot_parked_machine_with(coco2_t1_config());
    // Text mode, CSS=0, GM0 ($FF22 bit 4) set: enables true lowercase.
    m.bus.pia1.b.output = VDG_GM0_INTEXT;
    m.bus.write(SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    let fg = VDG_FIXED_PALETTE[13]; // ALPHANUMERIC BRIGHT GREEN
    let bg = VDG_FIXED_PALETTE[12]; // ALPHANUMERIC DARK GREEN

    // True lowercase swaps fg/bg relative to the normal non-inverse mapping:
    // "on" pixels draw in `bg`, "off" pixels draw in `fg` (see
    // `video.rs::resolve_alpha_cell`).
    let cell = sample_cell(&m.framebuffer, 0, 0, bg, fg);
    assert_eq!(
        cell,
        glyph_bits(&T1_LOWER_A_GLYPH),
        "GM0 set + INV clear + code<$20 must draw the true-lowercase 'a' glyph, colour-swapped"
    );
}

#[test]
fn t1_without_gm0_code_01_is_not_lowercase() {
    let mut m = boot_parked_machine_with(coco2_t1_config());
    m.bus.pia1.b.output = 0; // GM0 clear
    m.bus.write(SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];
    // Without GM0, code $01 draws the ordinary uppercase 'A' from
    // `MC6847T1_FONT[1]`, not the lowercase glyph.
    let cell = sample_cell(&m.framebuffer, 0, 0, fg, bg);
    assert_ne!(
        cell,
        glyph_bits(&T1_LOWER_A_GLYPH),
        "code $01 with GM0 clear must not draw the lowercase glyph"
    );
}

#[test]
fn inverse_uppercase_a_is_unaffected_by_t1_lowercase_mode() {
    // Byte $41 = code 1 ('A'), INV set. Plain MC6847 (no T1 knob at all) and
    // T1-with-GM0-clear each draw their own font's ordinary uppercase 'A'
    // with the ordinary INV colour swap — today's existing inverse-uppercase
    // behaviour, unaffected by the new lowercase code path. (The two chips'
    // glyphs occupy different rows within the cell — see `PLAIN_A_GLYPH` /
    // `T1_A_GLYPH` — so this does *not* assert the two renders are pixel-
    // identical to each other, only that neither took the lowercase path.)
    let inverse_a = CODE_A | INVERSE_BIT;

    let mut plain = boot_parked_machine_with(coco1_config());
    plain.bus.pia1.b.output = 0;
    plain.bus.write(SCREEN_BASE, inverse_a);
    plain.run_field();

    let mut t1 = boot_parked_machine_with(coco2_t1_config());
    t1.bus.pia1.b.output = 0; // GM0 clear
    t1.bus.write(SCREEN_BASE, inverse_a);
    t1.run_field();

    let fg = VDG_FIXED_PALETTE[13];
    let bg = VDG_FIXED_PALETTE[12];
    // INV set -> normal mapping swaps to (bg, fg): "on" pixels draw in `bg`.
    let plain_cell = sample_cell(&plain.framebuffer, 0, 0, bg, fg);
    let t1_cell = sample_cell(&t1.framebuffer, 0, 0, bg, fg);
    assert_eq!(
        plain_cell,
        glyph_bits(&PLAIN_A_GLYPH),
        "plain MC6847 inverse 'A' must draw its own font's uppercase glyph"
    );
    assert_eq!(
        t1_cell,
        glyph_bits(&T1_A_GLYPH),
        "T1-with-GM0-clear inverse 'A' must draw its own font's uppercase glyph, not lowercase"
    );
}

// --- CoCo 3 CoCo-compatible text: the GIME's own generator, not a VDG font ---
//
// A real CoCo 3 has no VDG chip at all: when INIT0 COCO is set, the GIME
// draws CoCo-compatible text with its own font ROM
// (`crate::font_gime::GIME_LOWRES_FONT`, `video::AlphaGenerator::Gime`), not
// either MC6847 font — even though `MachineConfig::vdg` is forced to
// `Mc6847` for a CoCo 3 by `MachineConfig::validate` (it just doesn't apply).
// Expected glyph bit patterns are copied from `src/font_gime.rs`'s
// `GIME_LOWRES_FONT`: 'O' at index 15, true-lowercase 'a' at index 64+1=65.

/// `GIME_LOWRES_FONT[15]` ('O').
const GIME_O_GLYPH: [u8; CELL_H] = [
    0x00, 0x38, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0x00, 0x00, 0x00, 0x00,
];
/// `GIME_LOWRES_FONT[64 + 1]` (true-lowercase 'a').
const GIME_LOWER_A_GLYPH: [u8; CELL_H] = [
    0x00, 0x00, 0x00, 0x38, 0x04, 0x3C, 0x44, 0x3C, 0x00, 0x00, 0x00, 0x00,
];

/// GIME 6-bit RGB register value for white (all three 2-bit channels `0b11`
/// — see `tests/render.rs`'s `GIME_WHITE_RGBA` doc comment).
const GIME_WHITE6: u8 = 0x3F;
/// `GIME::rgb_color(GIME_WHITE6)`.
const GIME_WHITE_RGBA: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
/// `GIME::rgb_color(0)` — the untouched-palette-register default.
const GIME_BLACK_RGBA: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];

/// Text/graphics base for the CoCo 3 tests below: SAM F0+F1 set moves the
/// display base to $0600 (page 3), keeping it clear of the parked `BRA *` at
/// $0000 — same trick as `tests/render.rs`'s
/// `text_renderer_follows_sam_page_register`.
const COCO3_SCREEN_BASE: u16 = 0x0600;

/// A CoCo 3 (default `MachineConfig`) parked on `BRA *`, forced into
/// CoCo-compatible text mode (INIT0 COCO bit), with the display base moved
/// to [`COCO3_SCREEN_BASE`] and a known white-on-black text palette (the
/// GIME's palette registers are otherwise zeroed = black-on-black, since
/// nothing here runs the ROM's cold-start palette init).
fn boot_parked_coco3() -> Machine {
    let rom = vec![0u8; 32 * 1024].into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.bus.gime.write_init0(init0::COCO);
    m.bus.write(0x0000, 0x20); // BRA
    m.bus.write(0x0001, 0xFE); // -2
    m.bus.gime.palette[TEXT_FG_INDEX] = GIME_WHITE6;
    // palette[TEXT_BG_INDEX] left at its zeroed default: black.
    m.bus.write(0xFFC7, 0); // SAM F0 set
    m.bus.write(0xFFC9, 0); // SAM F1 set -> display base $0600 (page 3)
    m
}

#[test]
fn coco3_compat_text_draws_gime_font_not_either_vdg_font() {
    let mut m = boot_parked_coco3();
    m.bus.pia1.b.output = 0; // text mode, CSS=0, GM0 clear
    m.bus.write(COCO3_SCREEN_BASE, CODE_O);
    m.run_field();

    let cell = sample_cell(&m.framebuffer, 0, 0, GIME_WHITE_RGBA, GIME_BLACK_RGBA);

    assert_eq!(
        cell,
        glyph_bits(&GIME_O_GLYPH),
        "CoCo 3 compat text must draw the GIME's own 'O' glyph (GIME_LOWRES_FONT)"
    );
    assert_ne!(
        cell,
        glyph_bits(&PLAIN_O_GLYPH),
        "must not draw the plain MC6847's 'O' — a CoCo 3 has no MC6847 at all"
    );
    assert_ne!(
        cell,
        glyph_bits(&T1_O_GLYPH),
        "must not draw the MC6847T1's 'O' — a CoCo 3 has no MC6847T1 at all"
    );
}

#[test]
fn coco3_compat_text_true_lowercase_uses_gime_lowercase_font() {
    let mut m = boot_parked_coco3();
    m.bus.pia1.b.output = VDG_GM0_INTEXT; // text mode, CSS=0, GM0 set
    m.bus.write(COCO3_SCREEN_BASE, CODE_A); // code=1, INV clear
    m.run_field();

    // True lowercase swaps fg/bg relative to the normal non-inverse mapping:
    // "on" pixels draw in the background colour (black), "off" pixels in
    // the foreground colour (white) — see `video.rs::resolve_alpha_cell`.
    let cell = sample_cell(&m.framebuffer, 0, 0, GIME_BLACK_RGBA, GIME_WHITE_RGBA);
    assert_eq!(
        cell,
        glyph_bits(&GIME_LOWER_A_GLYPH),
        "GM0 set + INV clear + code<$20 must draw the GIME's true-lowercase 'a' glyph, colour-swapped"
    );
}
