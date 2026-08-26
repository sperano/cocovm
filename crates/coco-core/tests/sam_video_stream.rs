//! Discrete MC6883 video-counter behavior for CoCo 1/2.

use coco_core::sam::SAMVideoAddressStream;
use coco_core::video::{ACTIVE_H, VDG_AG, decode_vdg_graphics};

const DISPLAY_BASE: u16 = 0x0400;
const RESET_BASE: u16 = 0x0800;
const GM_SHIFT: u8 = 4;
const SIXTEEN_SAMPLES: usize = 16;
const THIRTY_TWO_SAMPLES: usize = 32;
const SAM_V0: u8 = 0;
const SAM_V1: u8 = 1;
const SAM_V2: u8 = 2;
const SAM_V3: u8 = 3;
const SAM_V4: u8 = 4;
const SAM_V5: u8 = 5;
const SAM_V6: u8 = 6;
const SAM_V7_DMA: u8 = 7;

fn sampled_line(
    stream: &mut SAMVideoAddressStream,
    logical_base: usize,
    samples: usize,
) -> Vec<u16> {
    let addresses = (0..samples)
        .map(|sample| stream.sample(logical_base + sample))
        .collect();
    stream.horizontal_sync();
    addresses
}

fn range(start: u16, len: usize) -> Vec<u16> {
    (0..len).map(|offset| start + offset as u16).collect()
}

#[test]
fn da0_transitions_advance_once_and_numeric_skips_do_not_catch_up() {
    let mut stream = SAMVideoAddressStream::new(DISPLAY_BASE, SAM_V7_DMA);
    assert_eq!(stream.sample(0), DISPLAY_BASE);
    assert_eq!(
        stream.sample(0),
        DISPLAY_BASE,
        "repeated DA0 must not advance"
    );
    assert_eq!(stream.sample(3), DISPLAY_BASE + 1);
    assert_eq!(stream.sample(1), DISPLAY_BASE + 1, "same DA0 after a skip");
    assert_eq!(stream.sample(2), DISPLAY_BASE + 2);
}

#[test]
fn field_reset_reloads_the_base_and_both_divider_phases() {
    let mut x_stream = SAMVideoAddressStream::new(DISPLAY_BASE, SAM_V1);
    for line in 0..2 {
        sampled_line(&mut x_stream, line * SIXTEEN_SAMPLES, SIXTEEN_SAMPLES);
    }
    x_stream.reset(RESET_BASE);
    let x_starts: Vec<u16> = (0..4)
        .map(|line| sampled_line(&mut x_stream, line * SIXTEEN_SAMPLES, SIXTEEN_SAMPLES)[0])
        .collect();
    assert_eq!(
        x_starts,
        [RESET_BASE, RESET_BASE, RESET_BASE, RESET_BASE + 16]
    );

    let mut y_stream = SAMVideoAddressStream::new(DISPLAY_BASE, SAM_V2);
    sampled_line(&mut y_stream, 0, THIRTY_TWO_SAMPLES);
    y_stream.reset(RESET_BASE);
    let y_starts: Vec<u16> = (0..4)
        .map(|line| sampled_line(&mut y_stream, line * THIRTY_TWO_SAMPLES, THIRTY_TWO_SAMPLES)[0])
        .collect();
    assert_eq!(
        y_starts,
        [RESET_BASE, RESET_BASE, RESET_BASE, RESET_BASE + 32]
    );
}

#[test]
fn v1_hs_carry_repeats_each_sixteen_byte_half_row_three_times() {
    let mut stream = SAMVideoAddressStream::new(0, SAM_V1);
    let starts: Vec<u16> = (0..7)
        .map(|line| sampled_line(&mut stream, line * SIXTEEN_SAMPLES, SIXTEEN_SAMPLES)[0])
        .collect();
    assert_eq!(starts, [0, 0, 0, 16, 16, 16, 32]);
}

#[test]
fn v2_hs_carry_repeats_each_thirty_two_byte_row_three_times() {
    let mut stream = SAMVideoAddressStream::new(0, SAM_V2);
    let starts: Vec<u16> = (0..4)
        .map(|line| sampled_line(&mut stream, line * THIRTY_TWO_SAMPLES, THIRTY_TWO_SAMPLES)[0])
        .collect();
    assert_eq!(starts, [0, 0, 0, 32]);
}

#[test]
fn exact_mismatches_follow_the_sam_reset_and_carry_rules() {
    let mut v0 = SAMVideoAddressStream::new(0, SAM_V0);
    assert_eq!(sampled_line(&mut v0, 0, SIXTEEN_SAMPLES), range(0, 16));
    assert_eq!(sampled_line(&mut v0, 16, SIXTEEN_SAMPLES), range(0, 16));

    let mut v3 = SAMVideoAddressStream::new(0, SAM_V3);
    let mut duplicated_half = range(0, 16);
    duplicated_half.extend(range(0, 16));
    assert_eq!(
        sampled_line(&mut v3, 0, THIRTY_TWO_SAMPLES),
        duplicated_half
    );

    let mut next_duplicated_half = range(16, 16);
    next_duplicated_half.extend(range(16, 16));
    assert_eq!(
        sampled_line(&mut v3, THIRTY_TWO_SAMPLES, THIRTY_TWO_SAMPLES),
        next_duplicated_half
    );

    let mut v6 = SAMVideoAddressStream::new(0, SAM_V6);
    assert_eq!(sampled_line(&mut v6, 0, SIXTEEN_SAMPLES), range(0, 16));
    assert_eq!(sampled_line(&mut v6, 16, SIXTEEN_SAMPLES), range(0, 16));
}

#[test]
fn v7_dma_ignores_hs_and_continues_linearly() {
    let mut stream = SAMVideoAddressStream::new(DISPLAY_BASE, SAM_V7_DMA);
    assert_eq!(
        sampled_line(&mut stream, 0, THIRTY_TWO_SAMPLES),
        range(DISPLAY_BASE, THIRTY_TWO_SAMPLES)
    );
    assert_eq!(
        sampled_line(&mut stream, THIRTY_TWO_SAMPLES, THIRTY_TWO_SAMPLES),
        range(DISPLAY_BASE + 32, THIRTY_TWO_SAMPLES)
    );
}

#[test]
fn all_eight_stock_graphics_pairings_produce_contiguous_logical_rows() {
    const STOCK_V: [u8; 8] = [
        SAM_V1, SAM_V1, SAM_V2, SAM_V3, SAM_V4, SAM_V5, SAM_V6, SAM_V6,
    ];

    for (gm, video_mode) in STOCK_V.into_iter().enumerate() {
        let ff22 = VDG_AG | (gm as u8) << GM_SHIFT;
        let mode = decode_vdg_graphics(ff22);
        let lines_per_row = ACTIVE_H / mode.rows;
        let mut stream = SAMVideoAddressStream::new(DISPLAY_BASE, video_mode);

        for line in 0..ACTIVE_H {
            let logical_base = line / lines_per_row * mode.bytes_per_row;
            let actual = sampled_line(&mut stream, logical_base, mode.bytes_per_row);
            let expected = range(DISPLAY_BASE + logical_base as u16, mode.bytes_per_row);
            assert_eq!(
                actual, expected,
                "GM={gm:03b}, V={video_mode:03b}, line={line}"
            );
        }
    }
}
