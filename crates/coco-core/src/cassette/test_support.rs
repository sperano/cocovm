//! Test-only support for this workspace's cassette test suites
//! (`coco-core/tests/cassette.rs`, `coco-core/tests/cassette_wav.rs`, and
//! `coco-egui`'s UI tape tests): tape framing and FSK-encoding helpers that
//! were duplicated across those files before this module existed. `pub` (not
//! `pub(crate)`) because integration tests and the `coco-egui` crate see
//! `coco-core` as an external dependency; `#[doc(hidden)]` keeps it off the
//! published docs since it's not part of the real API surface.

use super::{
    Cassette, DAC_FULL_SCALE, LEADER, ONE_BIT_HIGH, ONE_BIT_LOW, SYNC, ZERO_BIT_HIGH, ZERO_BIT_LOW,
};

/// Comfortably past [`super::MOTOR_SPINUP_CYCLES`] — burning this many
/// cycles with the motor on always drains the spin-up countdown, so a test
/// can start driving the tape/record tap immediately afterwards.
pub const SPINUP_BURN_CYCLES: u32 = 600_000;

/// One framed tape block: leader, sync, type, length, payload, checksum (sum
/// of type + length + payload), trailer (Service Manual §5.10).
pub fn tape_block(block_type: u8, payload: &[u8]) -> Vec<u8> {
    let len = u8::try_from(payload.len()).unwrap();
    let mut block = vec![LEADER, SYNC, block_type, len];
    block.extend_from_slice(payload);
    let checksum = payload
        .iter()
        .fold(block_type.wrapping_add(len), |acc, &b| acc.wrapping_add(b));
    block.push(checksum);
    block.push(LEADER);
    block
}

/// Feed a byte stream into the deck's record tap as full-swing DAC
/// transitions at the ROM's measured tone timings — one full period per bit,
/// LSB first — mirroring what the ROM's CSAVE bit-bang writes to the DAC.
pub fn record_bytes_fsk(deck: &mut Cassette, bytes: &[u8]) {
    for &byte in bytes {
        for bit in 0..8 {
            let is_one = byte >> bit & 1 == 1;
            let (high, low) = if is_one {
                (ONE_BIT_HIGH, ONE_BIT_LOW)
            } else {
                (ZERO_BIT_HIGH, ZERO_BIT_LOW)
            };
            deck.record_dac(DAC_FULL_SCALE, true);
            deck.tick(high, true);
            deck.record_dac(0, true);
            deck.tick(low, true);
        }
    }
}
