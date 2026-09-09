//! Byte-for-byte comparison with the former allocating TV pipeline.
use std::borrow::Cow;

use super::*;

const TEST_HEIGHT: usize = 3;
const TEST_WIDTHS: [usize; 5] = [1, 5, 256, 640, 912];
const TEST_STRENGTHS: [u8; 4] = [0, 35, MAX_PCT, u8::MAX];
const TEST_SEEDS: [u32; 3] = [0, 7, u32::MAX];
const PATTERN_MULTIPLIER: usize = 73;
const PATTERN_OFFSET: usize = 19;
const PATTERN_PERIOD: usize = 1 << u8::BITS;

pub(super) fn patterned_frame(width: usize, height: usize) -> Vec<u8> {
    (0..width * height * PX)
        .map(|i| {
            i.wrapping_mul(PATTERN_MULTIPLIER)
                .wrapping_add(PATTERN_OFFSET)
                .wrapping_add(i / PATTERN_PERIOD) as u8
        })
        .collect()
}

/// Frozen pass composition and allocating stages from before buffer reuse.
/// The unchanged luma and noise math is covered directly in display_test.rs.
pub(super) fn previous_process(
    tv: TV,
    settings: TVSettings,
    seed: u32,
    width: usize,
    src: &[u8],
) -> Vec<u8> {
    let signal: Cow<[u8]> = match tv {
        TV::Color => Cow::Borrowed(src),
        TV::BW => {
            let mut out = src.to_vec();
            for px in out.chunks_exact_mut(PX) {
                let y = luma(px[0], px[1], px[2]);
                px[..3].fill(y);
            }
            Cow::Owned(out)
        }
    };
    let mut pixels = previous_blur_rows(width, &signal);
    if settings.noise_pct > 0 {
        noise_rows(settings.noise_pct, seed, &mut pixels);
    }
    if settings.scanline_pct == 0 {
        pixels
    } else {
        previous_expand_scanlines(settings.scanline_pct, width, &pixels)
    }
}

fn previous_expand_scanlines(scanline_pct: u8, width: usize, src: &[u8]) -> Vec<u8> {
    let row_len = width * PX;
    let scale = scanline_scale(scanline_pct);
    // +128 for round-to-nearest; scale ≤ 256 keeps the product in u16.
    let dark = |v: u8| ((u16::from(v) * scale + 128) >> 8) as u8;
    let mut out = Vec::with_capacity(src.len() * 2);
    for row in src.chunks_exact(row_len) {
        out.extend_from_slice(row);
        for px in row.chunks_exact(PX) {
            out.extend_from_slice(&[dark(px[0]), dark(px[1]), dark(px[2]), px[3]]);
        }
    }
    out
}

/// Horizontal bandwidth limit: a [`BLUR_TAPS`] FIR across each row only —
/// an analog signal band-limits per scanline, so rows stay separate while
/// detail smears along the line. Edges clamp; alpha passes through.
fn previous_blur_rows(width: usize, src: &[u8]) -> Vec<u8> {
    let row_len = width * PX;
    let mut out = Vec::with_capacity(src.len());
    for row in src.chunks_exact(row_len) {
        for x in 0..width {
            let window = |i: usize| &row[i * PX..][..PX];
            let prev = window(x.saturating_sub(1));
            let cur = window(x);
            let next = window((x + 1).min(width - 1));
            for c in 0..3 {
                let sum = u16::from(prev[c]) * BLUR_TAPS[0]
                    + u16::from(cur[c]) * BLUR_TAPS[1]
                    + u16::from(next[c]) * BLUR_TAPS[2];
                // +half for round-to-nearest rather than truncation.
                out.push(((sum + BLUR_SUM / 2) / BLUR_SUM) as u8);
            }
            out.push(cur[3]);
        }
    }
    out
}

#[test]
fn reused_buffers_match_allocating_pipeline_for_all_tv_settings() {
    let mut processor = Processor::default();
    for width in TEST_WIDTHS {
        let src = patterned_frame(width, TEST_HEIGHT);
        for tv in [TV::Color, TV::BW] {
            for scanline_pct in TEST_STRENGTHS {
                for noise_pct in TEST_STRENGTHS {
                    let settings = TVSettings {
                        scanline_pct,
                        noise_pct,
                        ..TVSettings::default()
                    };
                    for seed in TEST_SEEDS {
                        let expected = previous_process(tv, settings, seed, width, &src);
                        let frame = processor.process(Display::TV(tv), settings, seed, width, &src);
                        assert_eq!(
                            frame.pixels, expected,
                            "{tv:?}, {settings:?}, seed={seed}, width={width}"
                        );
                        let rows = if scanline_pct == 0 { 1 } else { SCANLINE_ROWS };
                        assert_eq!((frame.width, frame.height), (width, TEST_HEIGHT * rows));
                    }
                }
            }
        }
    }
}

fn storage(processor: &Processor) -> [(usize, *const u8); 2] {
    [
        (processor.signal.capacity(), processor.signal.as_ptr()),
        (processor.output.capacity(), processor.output.as_ptr()),
    ]
}

#[test]
fn buffers_keep_capacity_and_addresses_across_settings_and_size_changes() {
    let mut processor = Processor::default();
    let width = *TEST_WIDTHS.last().unwrap();
    let src = patterned_frame(width, TEST_HEIGHT);
    processor.process(Display::TV(TV::BW), TVSettings::default(), 0, width, &src);
    let warmed = storage(&processor);
    for width in TEST_WIDTHS.into_iter().rev() {
        let src = patterned_frame(width, TEST_HEIGHT);
        for tv in [TV::Color, TV::BW] {
            for scanline_pct in TEST_STRENGTHS {
                let settings = TVSettings {
                    scanline_pct,
                    ..TVSettings::default()
                };
                for seed in TEST_SEEDS {
                    processor.process(Display::TV(tv), settings, seed, width, &src);
                    assert_eq!(storage(&processor), warmed);
                }
            }
        }
    }
}

#[test]
fn monitors_borrow_source_without_allocating_or_modifying_tv_buffers() {
    let mut processor = Processor::default();
    let width = TEST_WIDTHS[1];
    let src = patterned_frame(width, TEST_HEIGHT);
    for warm_tv in [false, true] {
        if warm_tv {
            processor.process(Display::TV(TV::BW), TVSettings::default(), 0, width, &src);
        }
        let retained = (processor.signal.clone(), processor.output.clone());
        let before = storage(&processor);
        for monitor in [MonitorType::RGB, MonitorType::Composite] {
            let frame = processor.process(
                Display::Monitor(monitor),
                TVSettings::default(),
                0,
                width,
                &src,
            );
            assert!(std::ptr::eq(frame.pixels, src.as_slice()));
            assert_eq!(storage(&processor), before);
            assert_eq!(
                (&processor.signal, &processor.output),
                (&retained.0, &retained.1)
            );
        }
    }
}
