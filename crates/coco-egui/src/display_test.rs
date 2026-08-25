use coco_core::{MachineConfig, MachineVariant, MonitorType};
use eframe::egui;

use super::*;

/// One RGBA test frame (5×1): black, white, and the three saturated
/// primaries.
const FRAME: [u8; 20] = [
    0, 0, 0, 255, // black
    255, 255, 255, 255, // white
    255, 0, 0, 255, // red
    0, 255, 0, 255, // green
    0, 0, 255, 255, // blue
];
const FRAME_W: usize = FRAME.len() / 4;

/// A `w`×`h` frame of one uniform color.
fn uniform(w: usize, h: usize, rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat(w * h)
}

/// Default settings minus the RF noise — the deterministic baseline the
/// structural tests run at (noise gets its own direct tests below).
fn quiet() -> TVSettings {
    TVSettings {
        noise_pct: 0,
        ..TVSettings::default()
    }
}

/// What [`expand_scanlines`] does to one byte at the given strength.
fn dark(v: u8, pct: u8) -> u8 {
    ((u16::from(v) * scanline_scale(pct) + 128) >> 8) as u8
}

// ---- luma ----

#[test]
fn luma_black_and_white_are_fixed_points() {
    assert_eq!(luma(0, 0, 0), 0);
    // Weights sum to 1.0, so full white round-trips exactly.
    assert_eq!(luma(255, 255, 255), 255);
}

#[test]
fn luma_greys_are_fixed_points() {
    // r=g=b is its own luminance; decode/encode cancel except for ±1 quantization.
    for v in [1u8, 17, 128, 200, 254] {
        assert!(
            (luma(v, v, v) as i32 - v as i32).abs() <= 1,
            "grey {v} must survive the round trip, got {}",
            luma(v, v, v)
        );
    }
}

#[test]
fn luma_saturated_primaries_hit_their_rec601_weights() {
    // Expected: (255·w^(1/γ)).round(); green is far brighter than blue. ±1 quantization.
    for (channel, expected) in [((255, 0, 0), 147), ((0, 255, 0), 200), ((0, 0, 255), 95)] {
        let (r, g, b) = channel;
        assert!(
            (luma(r, g, b) as i32 - expected).abs() <= 1,
            "luma{channel:?} = {}, expected ~{expected}",
            luma(r, g, b)
        );
    }
}

// ---- individual chain stages, tested directly for exact values ----

#[test]
fn blur_smears_along_the_row_only() {
    // White impulse in top row; 1-2-1 kernel spreads it (64/128/64) but must not bleed into the
    // row below.
    #[rustfmt::skip]
    let src: Vec<u8> = vec![
        0, 0, 0, 255,   255, 255, 255, 255,   0, 0, 0, 255,
        0, 0, 0, 255,   0, 0, 0, 255,         0, 0, 0, 255,
    ];
    let out = blur_rows(3, &src);
    let expect_top = [64u8, 128, 64];
    for (x, &expected) in expect_top.iter().enumerate() {
        assert_eq!(
            &out[x * 4..x * 4 + 4],
            [expected, expected, expected, 255],
            "top row pixel {x}"
        );
    }
    assert_eq!(
        &out[12..],
        [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
        "no vertical bleed"
    );
}

#[test]
fn expand_scanlines_interleaves_bright_and_dimmed_rows() {
    let pct = DEFAULT_SCANLINE_PCT;
    let rgba = [200u8, 120, 40, 255];
    let src = uniform(4, 2, rgba);
    let out = expand_scanlines(pct, 4, &src);
    assert_eq!(out.len(), src.len() * 2);
    let dim = [
        dark(rgba[0], pct),
        dark(rgba[1], pct),
        dark(rgba[2], pct),
        255,
    ];
    for (i, row) in out.chunks_exact(4 * 4).enumerate() {
        let expected = if i % 2 == 0 { &rgba } else { &dim };
        for px in row.chunks_exact(4) {
            assert_eq!(px, expected, "row {i}");
        }
    }
    // The dark half really is darker, and not black.
    assert!(dim[0] < rgba[0] && dim[0] > 0);
}

#[test]
fn scanline_strength_endpoints() {
    // 0% is a separate skip path in `process`, covered elsewhere.
    let src = uniform(4, 1, [200, 120, 40, 255]);
    let out = expand_scanlines(100, 4, &src);
    for px in out[4 * 4..].chunks_exact(4) {
        assert_eq!(px, [0, 0, 0, 255]);
    }
}

#[test]
fn noise_is_bounded_luminance_jitter() {
    let noise_pct = 50u8;
    let amp = i32::from(noise_pct) * NOISE_FULL / 100;
    let v = 128u8;
    let mut bytes = uniform(8, 8, [v, v, v, 255]);
    noise_rows(noise_pct, 7, &mut bytes);
    let mut saw_change = false;
    for px in bytes.chunks_exact(4) {
        assert!(px[0] == px[1] && px[1] == px[2], "luminance-only jitter");
        assert_eq!(px[3], 255, "alpha untouched");
        let d = (i32::from(px[0]) - i32::from(v)).abs();
        assert!(d <= amp, "deviation {d} exceeds amplitude {amp}");
        saw_change |= d != 0;
    }
    assert!(saw_change, "50% noise must visibly perturb the frame");
}

#[test]
fn noise_varies_with_the_seed_and_is_reproducible() {
    let src = uniform(8, 8, [128, 128, 128, 255]);
    let (mut a, mut b, mut c) = (src.clone(), src.clone(), src);
    noise_rows(30, 1, &mut a);
    noise_rows(30, 1, &mut b);
    noise_rows(30, 2, &mut c);
    assert_eq!(a, b, "same seed, same snow");
    assert_ne!(a, c, "a new seed must re-roll the snow");
}

// ---- the assembled chain ----

#[test]
fn process_is_identity_for_monitors() {
    for monitor in [MonitorType::RGB, MonitorType::Composite] {
        let frame = process(
            Display::Monitor(monitor),
            TVSettings::default(),
            0,
            FRAME_W,
            &FRAME,
        );
        assert_eq!(
            &*frame.pixels, FRAME,
            "Monitor({monitor:?}) must pass through"
        );
        assert!(
            matches!(frame.pixels, std::borrow::Cow::Borrowed(_)),
            "a monitor must not copy the framebuffer"
        );
        assert_eq!((frame.width, frame.height), (FRAME_W, 1));
    }
}

#[test]
fn process_tv_doubles_height_for_scanlines() {
    let src = uniform(8, 4, [200, 120, 40, 255]);
    let frame = process(Display::TV(TV::Color), quiet(), 0, 8, &src);
    assert_eq!((frame.width, frame.height), (8, 8));
}

#[test]
fn process_skips_doubling_at_zero_scanlines() {
    let src = uniform(8, 4, [200, 120, 40, 255]);
    let settings = TVSettings {
        scanline_pct: 0,
        ..quiet()
    };
    let frame = process(Display::TV(TV::Color), settings, 0, 8, &src);
    assert_eq!((frame.width, frame.height), (8, 4));
}

#[test]
fn process_bw_output_is_grey_everywhere() {
    // Blur and scanlines scale channels identically, so grey stays grey after luma collapse.
    let src = uniform(8, 4, [255, 0, 0, 255]);
    let frame = process(Display::TV(TV::BW), quiet(), 0, 8, &src);
    for px in frame.pixels.chunks_exact(4) {
        assert!(px[0] == px[1] && px[1] == px[2], "must stay grey");
        assert_eq!(px[3], 255);
    }
    let y = luma(255, 0, 0);
    let center = &frame.pixels[(4 * 8 + 4) * 4..][..4];
    assert_eq!(center[0], y, "bright-row pixel carries the luma");
}

// ---- Display resolution table ----

/// The whole design table: which signal path each display resolves to, per
/// variant. A monitor passes through even with no port so `validate` can
/// reject it.
#[test]
fn to_monitor_covers_the_design_table() {
    use MachineVariant::{Coco1, Coco2, Coco3};
    for variant in [Coco1, Coco2, Coco3] {
        for monitor in [MonitorType::RGB, MonitorType::Composite] {
            assert_eq!(Display::Monitor(monitor).to_monitor(variant), Some(monitor));
        }
        let tv_path = match variant {
            Coco3 => Some(MonitorType::Composite),
            Coco1 | Coco2 => None,
        };
        for tv in [TV::Color, TV::BW] {
            assert_eq!(Display::TV(tv).to_monitor(variant), tv_path);
        }
    }
}

#[test]
fn monitors_sample_nearest_and_tvs_bilinear() {
    for monitor in [MonitorType::RGB, MonitorType::Composite] {
        assert_eq!(
            texture_options(Display::Monitor(monitor)),
            egui::TextureOptions::NEAREST
        );
    }
    for tv in [TV::Color, TV::BW] {
        assert_eq!(
            texture_options(Display::TV(tv)),
            egui::TextureOptions::LINEAR
        );
    }
}

#[test]
fn choices_offer_monitors_only_where_a_port_exists() {
    assert_eq!(Display::choices(MachineVariant::Coco3).len(), 4);
    for variant in [MachineVariant::Coco1, MachineVariant::Coco2] {
        assert_eq!(
            Display::choices(variant),
            &[Display::TV(TV::Color), Display::TV(TV::BW)]
        );
    }
}

#[test]
fn from_config_round_trips_through_to_monitor_where_it_can() {
    // Exact except a CoCo 3 TV, which serializes as composite.
    let mut config = MachineConfig::default();
    assert_eq!(
        Display::from_config(&config),
        Display::Monitor(MonitorType::RGB)
    );
    config.monitor = None;
    assert_eq!(Display::from_config(&config), Display::TV(TV::Color));
}
