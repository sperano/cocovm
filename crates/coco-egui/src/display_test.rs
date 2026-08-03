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

/// A 4×2 frame of one uniform color — invariant under the bandwidth
/// limit (blurring a constant is the constant), so TV effects on it show
/// only the color transform.
fn uniform_frame(rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat(8)
}

#[test]
fn luma_black_and_white_are_fixed_points() {
    assert_eq!(luma(0, 0, 0), 0);
    // The weights sum to 1.0, so full white round-trips exactly through
    // the linear-light decode/encode.
    assert_eq!(luma(255, 255, 255), 255);
}

#[test]
fn luma_greys_are_fixed_points() {
    // r = g = b already is its own luminance: the weighted sum reduces to
    // the one channel value, and decode/encode cancel. The encode table's
    // quantization may move a level by at most one.
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
    // (255·w^(1/γ)).round() for w = 0.299/0.587/0.114, γ = 2.2 — the
    // linear-light weighting: green is by far the brightest, blue the
    // darkest, the whole point of weighting instead of averaging. ±1 for
    // the encode table's quantization.
    for (channel, expected) in [((255, 0, 0), 147), ((0, 255, 0), 200), ((0, 0, 255), 95)] {
        let (r, g, b) = channel;
        assert!(
            (luma(r, g, b) as i32 - expected).abs() <= 1,
            "luma{channel:?} = {}, expected ~{expected}",
            luma(r, g, b)
        );
    }
}

/// What [`expand_scanlines`] does to one byte at the given strength.
fn dark(v: u8, pct: u8) -> u8 {
    ((u16::from(v) * scanline_scale(pct) + 128) >> 8) as u8
}

/// Default settings minus the RF noise — the deterministic baseline the
/// structural tests run at (noise gets its own tests below).
fn quiet() -> TVSettings {
    TVSettings {
        noise_pct: 0,
        ..TVSettings::default()
    }
}

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
            frame.pixels, FRAME,
            "Monitor({monitor:?}) must pass through"
        );
        assert_eq!((frame.width, frame.height), (FRAME_W, 1));
    }
}

#[test]
fn process_color_tv_scanline_doubles_a_uniform_frame() {
    // Uniform color: the bandwidth limit has nothing to smear, so what's
    // left is exactly the scanline structure — height doubles, even rows
    // keep the color, odd rows are its dimmed copy.
    let settings = quiet();
    let rgba = [200u8, 120, 40, 255];
    let src = uniform_frame(rgba);
    let frame = process(Display::TV(TV::Color), settings, 0, 4, &src);
    assert_eq!((frame.width, frame.height), (4, 4));
    let dim: Vec<u8> = rgba[..3]
        .iter()
        .map(|&v| dark(v, settings.scanline_pct))
        .chain([255])
        .collect();
    for (i, row) in frame.pixels.chunks_exact(4 * 4).enumerate() {
        let expected = if i % 2 == 0 { &rgba[..] } else { &dim[..] };
        for px in row.chunks_exact(4) {
            assert_eq!(px, expected, "row {i}");
        }
    }
    // The dark half really is darker, and not black.
    assert!(dim[0] < rgba[0] && dim[0] > 0);
}

#[test]
fn scanline_strength_endpoints() {
    let rgba = [200u8, 120, 40, 255];
    let src = uniform_frame(rgba);

    // 0% disables the pass entirely: no doubling, bytes untouched.
    let off = process(
        Display::TV(TV::Color),
        TVSettings {
            scanline_pct: 0,
            ..quiet()
        },
        0,
        4,
        &src,
    );
    assert_eq!(off.height, 2, "0% must not scanline-double");
    assert_eq!(off.pixels, src);

    // 100% is black gaps.
    let full = process(
        Display::TV(TV::Color),
        TVSettings {
            scanline_pct: 100,
            ..quiet()
        },
        0,
        4,
        &src,
    );
    for row in full.pixels.chunks_exact(4 * 4).skip(1).step_by(2) {
        for px in row.chunks_exact(4) {
            assert_eq!(px, [0, 0, 0, 255]);
        }
    }
}

#[test]
fn process_bw_greys_every_pixel_and_keeps_alpha() {
    let settings = quiet();
    let src = uniform_frame([255, 0, 0, 255]);
    let frame = process(Display::TV(TV::BW), settings, 0, 4, &src);
    let y = luma(255, 0, 0);
    let dark_y = dark(y, settings.scanline_pct);
    for (i, row) in frame.pixels.chunks_exact(4 * 4).enumerate() {
        let expected = if i % 2 == 0 { y } else { dark_y };
        for px in row.chunks_exact(4) {
            assert_eq!(px, [expected, expected, expected, 255], "row {i}");
        }
    }
}

#[test]
fn tvs_bandwidth_limit_smears_along_the_row_only() {
    // A 3×2 frame: a white impulse in the top row, black bottom row. The
    // 1-2-1 kernel spreads the impulse to its row neighbors (64/128/64
    // with edge clamp) and must leak nothing into the source row below —
    // scanlines are separate signals. With the scanline pass, source row 0
    // lands in output rows 0 (bright) and 1 (dark); source row 1 in output
    // rows 2/3, which must stay black.
    #[rustfmt::skip]
    let src: Vec<u8> = vec![
        0, 0, 0, 255,   255, 255, 255, 255,   0, 0, 0, 255,
        0, 0, 0, 255,   0, 0, 0, 255,         0, 0, 0, 255,
    ];
    let settings = quiet();
    let frame = process(Display::TV(TV::Color), settings, 0, 3, &src);
    assert_eq!(frame.height, 4);
    let rows: Vec<&[u8]> = frame.pixels.chunks_exact(3 * 4).collect();
    let expect_bright = [64u8, 128, 64];
    for (x, &expected) in expect_bright.iter().enumerate() {
        assert_eq!(
            &rows[0][x * 4..x * 4 + 4],
            [expected, expected, expected, 255],
            "bright row pixel {x}"
        );
        assert_eq!(
            rows[1][x * 4],
            dark(expected, settings.scanline_pct),
            "dark row pixel {x}"
        );
    }
    for row in &rows[2..] {
        assert_eq!(
            *row,
            [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
            "no vertical bleed"
        );
    }
}

/// A mid-grey frame processed with noise only (no scanlines): every output
/// byte stays within the amplitude bound, and the noise is luminance-only
/// (equal offset on R, G, and B).
#[test]
fn noise_is_bounded_luminance_jitter() {
    let noise_pct = 50u8;
    let amp = i32::from(noise_pct) * NOISE_FULL / 100;
    let settings = TVSettings {
        scanline_pct: 0,
        noise_pct,
    };
    let v = 128u8;
    let src = uniform_frame([v, v, v, 255]);
    let frame = process(Display::TV(TV::BW), settings, 7, 4, &src);
    let mut saw_change = false;
    for px in frame.pixels.chunks_exact(4) {
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
    let settings = TVSettings {
        scanline_pct: 0,
        noise_pct: 30,
    };
    let src = uniform_frame([128, 128, 128, 255]);
    let a = process(Display::TV(TV::Color), settings, 1, 4, &src);
    let b = process(Display::TV(TV::Color), settings, 1, 4, &src);
    let c = process(Display::TV(TV::Color), settings, 2, 4, &src);
    assert_eq!(a.pixels, b.pixels, "same seed, same snow");
    assert_ne!(a.pixels, c.pixels, "a new seed must re-roll the snow");
}

/// The whole design table: which signal path each display resolves to, per
/// variant. A monitor passes through even where no port exists — that's how
/// `MachineConfig::validate` gets to reject it with the real reason.
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
    // A validated config's monitor implies the display exactly — except a
    // CoCo 3 TV, which serializes as composite (`from_config`'s doc).
    let mut config = MachineConfig::default();
    assert_eq!(
        Display::from_config(&config),
        Display::Monitor(MonitorType::RGB)
    );
    config.monitor = None;
    assert_eq!(Display::from_config(&config), Display::TV(TV::Color));
}
