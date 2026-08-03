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

#[test]
fn apply_is_identity_for_monitors() {
    for monitor in [MonitorType::RGB, MonitorType::Composite] {
        let mut bytes = FRAME;
        apply(Display::Monitor(monitor), FRAME_W, &mut bytes);
        assert_eq!(bytes, FRAME, "Monitor({monitor:?}) must pass through");
    }
}

#[test]
fn apply_color_tv_preserves_a_uniform_frame() {
    // Uniform color: the bandwidth limit has nothing to smear, so the
    // color TV — no color transform of its own yet — is an exact identity.
    let frame = uniform_frame([200, 120, 40, 255]);
    let mut bytes = frame.clone();
    apply(Display::TV(TV::Color), 4, &mut bytes);
    assert_eq!(bytes, frame);
}

#[test]
fn apply_bw_greys_every_pixel_and_keeps_alpha() {
    let frame = uniform_frame([255, 0, 0, 255]);
    let mut bytes = frame.clone();
    apply(Display::TV(TV::BW), 4, &mut bytes);
    let y = luma(255, 0, 0);
    for px in bytes.chunks_exact(4) {
        assert_eq!(px, [y, y, y, 255]);
    }
}

#[test]
fn tvs_bandwidth_limit_smears_along_the_row_only() {
    // A 3×2 frame: a white impulse in the top row, black bottom row. The
    // 1-2-1 kernel spreads the impulse to its row neighbors (64/128/64
    // with edge clamp) and must leak nothing into the row below —
    // scanlines are separate signals.
    #[rustfmt::skip]
    let mut bytes: Vec<u8> = vec![
        0, 0, 0, 255,   255, 255, 255, 255,   0, 0, 0, 255,
        0, 0, 0, 255,   0, 0, 0, 255,         0, 0, 0, 255,
    ];
    apply(Display::TV(TV::Color), 3, &mut bytes);
    let expect_top = [64u8, 128, 64];
    for (x, &expected) in expect_top.iter().enumerate() {
        assert_eq!(
            &bytes[x * 4..x * 4 + 4],
            [expected, expected, expected, 255],
            "top row pixel {x}"
        );
    }
    assert_eq!(
        &bytes[12..],
        [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
        "no vertical bleed"
    );
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
