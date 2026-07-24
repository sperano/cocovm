//! Composite-monitor palette decoding (`GIME::color`): the hand-measured
//! MAME `get_composite_color` tables, BPI (burst phase invert) table
//! selection, and MOCH (monochrome-on-composite) greyscale averaging.
//! Register values and tables follow MAME `src/mame/trs/gime.cpp`.

use coco_core::gime::{GIME, vmode};
use coco_core::gime_video::render_field;
use coco_core::video::BYTES_PER_PIXEL;
use coco_core::MonitorType;

fn px(fb: &[u8], fb_w: usize, x: usize, y: usize) -> [u8; 4] {
    let i = (y * fb_w + x) * BYTES_PER_PIXEL;
    fb[i..i + 4].try_into().unwrap()
}

#[test]
fn composite_decode_grey_anchors() {
    let g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    assert_eq!(g.color(0x00), [0x00, 0x00, 0x00, 0xFF]);
    assert_eq!(g.color(0x10), [0x2d, 0x2d, 0x2d, 0xFF]);
    assert_eq!(g.color(0x20), [0x74, 0x74, 0x74, 0xFF]);
    assert_eq!(g.color(0x30), [0xfd, 0xfd, 0xfe, 0xFF]);
}

#[test]
fn composite_decode_hue_and_bpi() {
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    assert_eq!(g.color(0x01), [0x00, 0x4c, 0x00, 0xFF]);

    g.vmode = vmode::BPI;
    assert_eq!(g.color(0x01), [0x5a, 0x0e, 0x5a, 0xFF]);
}

#[test]
fn composite_moch_averages_channels() {
    // Normal-table index 0x01 = 0x004c00 -> r=0x00, g=0x4c (76), b=0x00.
    // (0 + 76 + 0) / 3 = 25 (integer division) = 0x19.
    let mut g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    g.vmode = vmode::MOCH;
    assert_eq!(g.color(0x01), [0x19, 0x19, 0x19, 0xFF]);
}

#[test]
fn rgb_monitor_ignores_bpi_and_moch() {
    let g = GIME {
        monitor: MonitorType::RGB,
        vmode: vmode::BPI | vmode::MOCH,
        ..GIME::new()
    };
    assert_eq!(g.color(0x10), GIME::rgb_color(0x10));
}

#[test]
fn eou_greyscale_regression() {
    // NitrOS-9 EOU's gshell greyscale desktop programs palette regs 0/0x10/
    // 0x20/0x30. On a composite monitor these must decode to achromatic,
    // strictly increasing brightness (not black/green/red/yellow as an
    // RGB-only decode would render them).
    let g = GIME {
        monitor: MonitorType::Composite,
        ..GIME::new()
    };
    let regs = [0x00u8, 0x10, 0x20, 0x30];
    let colors: Vec<[u8; 4]> = regs.iter().map(|&r| g.color(r)).collect();
    // The hand-measured table isn't perfectly achromatic at every entry
    // (0x30 = 0xfdfdfe, one LSB off on blue), so allow a 1-count tolerance
    // rather than exact channel equality.
    const GREY_TOLERANCE: u8 = 1;
    for c in &colors {
        assert!(c[0].abs_diff(c[1]) <= GREY_TOLERANCE, "achromatic: r ~= g ({c:?})");
        assert!(c[1].abs_diff(c[2]) <= GREY_TOLERANCE, "achromatic: g ~= b ({c:?})");
    }
    assert!(colors[0][0] < colors[1][0]);
    assert!(colors[1][0] < colors[2][0]);
    assert!(colors[2][0] < colors[3][0]);
}

/// Physical video base used by the render-path test, expressed as the
/// $FF9D/$FF9E value (mirrors `render_gime.rs`'s `VOFF`/`BASE` convention).
const VOFF: u16 = 0x1000; // physical $8000 (VOFF × 8)
const BASE: usize = (VOFF as usize) << 3;
/// 128K of physical RAM, like the base machine.
const RAM_LEN: usize = 0x20000;
/// $FF98 for hi-res text: BP=0, LPR=%011 -> 8 lines per character row.
const TEXT_LPR8: u8 = 0x03;
/// $FF99 for 40-column text without attributes: HRES=%001.
const VRES_TEXT40: u8 = 0x04;

#[test]
fn render_text_routes_through_composite_decode() {
    let mut g = GIME::new();
    g.monitor = MonitorType::Composite;
    g.vmode = TEXT_LPR8;
    g.vres = VRES_TEXT40;
    g.vertical_offset = VOFF;
    // Palette reg 1 (foreground for attribute-less text) holds a distinctive
    // 6-bit value; palette reg 0 (background) stays 0 (black either way).
    g.palette[1] = 0x01;

    let mut ram = vec![0u8; RAM_LEN];
    ram[BASE] = b'A';

    let mut fb = Vec::new();
    let (fb_w, _) = render_field(&g, &ram, false, &mut fb);

    // 'A' row 0 is 0x10: native pixel 3 lit -> foreground (palette reg 1).
    // 40 columns is a wide canonical mode: xscale 2, no side border, body
    // starts at canvas row 25 (LPF=%00).
    let expected_fg = g.color(0x01);
    assert_ne!(
        expected_fg,
        GIME::rgb_color(0x01),
        "test is only meaningful if composite and RGB decode differ here"
    );
    assert_eq!(px(&fb, fb_w, 3 * 2, 25), expected_fg);
}
