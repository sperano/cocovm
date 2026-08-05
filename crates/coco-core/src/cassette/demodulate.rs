//! Demodulation: turning a raw DAC transition capture back into decoded tape
//! bytes. Split out of `cassette.rs` (which keeps the constants both this and
//! [`crate::cassette_wav`] need, [`super::ZERO_BIT_PERIOD`]/
//! [`super::ONE_BIT_PERIOD`]/[`super::LEADER`]/[`super::SYNC`]) to keep that
//! file under the project's size guideline; everything here is demodulation-
//! only and has no callers outside [`Cassette::finalize_recording`]
//! ([`super::Cassette::finalize_recording`]).

use super::{LEADER, ONE_BIT_PERIOD, SYNC, Transition, ZERO_BIT_PERIOD};

/// Demodulation decision boundary between the two measured periods
/// (midpoint of 455 and 793): a full period at or below this is a 1 bit.
const BIT_PERIOD_THRESHOLD: u64 = (ZERO_BIT_PERIOD as u64 + ONE_BIT_PERIOD as u64) / 2;

/// A period twice the 0-bit's is no tone at all: a discontinuity (motor
/// spin-up glitch, inter-block artifact). The demodulator drops sync and
/// re-hunts for a leader when it sees one.
const PERIOD_BREAK: u64 = 2 * ZERO_BIT_PERIOD as u64;

/// Demodulate a DAC transition capture into the decoded byte stream.
///
/// Crossings of the waveform midpoint mark the tone phase; the time between
/// consecutive *rising* crossings is one full tone cycle = one bit
/// ([`BIT_PERIOD_THRESHOLD`] splits 1 from 0) — see [`capture_to_bits`]. Byte
/// alignment is recovered the way the BIOS does it: hunt bit-by-bit for
/// [`LEADER`] bytes then the [`SYNC`], then read the block structure (type,
/// length, payload, checksum, trailer) byte-aligned, and go back to hunting
/// — see [`bits_to_bytes`] — so a glitch between blocks only costs
/// re-syncing on the next leader, exactly like real tape.
pub fn demodulate(capture: &[Transition]) -> Vec<u8> {
    bits_to_bytes(capture_to_bits(capture))
}

/// Crossing-detect a DAC transition capture into a demodulated bit stream:
/// one entry per detected tone cycle, `None` marking a discontinuity (period
/// too long to be a tone — a motor spin-up glitch or inter-block artifact).
fn capture_to_bits(capture: &[Transition]) -> Vec<Option<bool>> {
    let Some(max) = capture.iter().map(|t| t.level).max() else {
        return Vec::new();
    };
    if max == 0 {
        return Vec::new();
    }
    let mid = max / 2;

    let mut bits: Vec<Option<bool>> = Vec::new();
    let mut side = capture[0].level > mid;
    // A capture that starts on the high side starts mid-cycle: count the
    // first period from its first sample, or the opening bit is lost.
    let mut last_rise: Option<u64> = side.then_some(capture[0].cycle);
    let mut last_fall: Option<u64> = None;
    for t in &capture[1..] {
        let new_side = t.level > mid;
        if new_side && !side {
            if let Some(prev) = last_rise {
                let period = t.cycle - prev;
                bits.push(if period > PERIOD_BREAK {
                    None
                } else {
                    Some(period <= BIT_PERIOD_THRESHOLD)
                });
            }
            last_rise = Some(t.cycle);
        } else if !new_side && side {
            last_fall = Some(t.cycle);
        }
        side = new_side;
    }
    // The recording stops with the last tone cycle's closing rise never
    // written (the ROM turns the motor off right after the final byte), so
    // the stream's very last bit dangles. Salvage it from its half-width —
    // without it the tape loses the EOF block's trailer and the ROM's
    // BITIN ($A755) polls forever for the missing edge on playback.
    if let (Some(rise), Some(fall)) = (last_rise, last_fall)
        && fall > rise
        && fall - rise <= PERIOD_BREAK / 2
    {
        bits.push(Some(2 * (fall - rise) <= BIT_PERIOD_THRESHOLD));
    }
    bits
}

/// How far through a locked (byte-aligned) block [`bits_to_bytes`]'s reader is.
enum BlockState {
    /// Bit-level hunt: sliding window looking for LEADER runs, then SYNC.
    Hunt,
    /// Byte-aligned after a sync: `seen` bytes read so far; `total` is
    /// type + length + payload + checksum + trailer, known once the
    /// length byte (the second one) arrives.
    Locked { seen: usize, total: usize },
}

/// Block bytes besides the payload: type, length, checksum, trailer $55.
const BLOCK_OVERHEAD: usize = 4;

/// Recover byte alignment from a demodulated bit stream ([`capture_to_bits`])
/// the way the BIOS does it: hunt bit-by-bit for [`LEADER`] runs then a
/// [`SYNC`], then read one block byte-aligned (type, length, payload,
/// checksum, trailer) before returning to hunting. A `None` bit (a capture
/// discontinuity) drops any in-progress hunt/lock and starts over.
fn bits_to_bytes(bits: Vec<Option<bool>>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut state = BlockState::Hunt;
    let mut window: u8 = 0;
    let mut window_bits = 0u32;
    let mut leader_count = 0usize;
    for bit in bits {
        let Some(bit) = bit else {
            state = BlockState::Hunt;
            window = 0;
            window_bits = 0;
            leader_count = 0;
            continue;
        };
        window = window >> 1 | u8::from(bit) << 7; // LSB arrives first
        window_bits += 1;
        match state {
            BlockState::Hunt => {
                if window_bits < 8 {
                    continue;
                }
                if window == LEADER {
                    leader_count += 1;
                    window_bits = 0;
                } else if window == SYNC {
                    out.extend(std::iter::repeat_n(LEADER, leader_count));
                    out.push(SYNC);
                    leader_count = 0;
                    window_bits = 0;
                    state = BlockState::Locked {
                        seen: 0,
                        total: usize::MAX,
                    };
                }
            }
            BlockState::Locked {
                ref mut seen,
                ref mut total,
            } => {
                if window_bits < 8 {
                    continue;
                }
                out.push(window);
                window_bits = 0;
                *seen += 1;
                if *seen == 2 {
                    // `window` is the length byte: the payload size.
                    *total = usize::from(window) + BLOCK_OVERHEAD;
                }
                if *seen >= *total {
                    state = BlockState::Hunt;
                }
            }
        }
    }
    // A trailing leader run with no sync after it (e.g. the stream ended
    // mid-gap) is still tape content.
    out.extend(std::iter::repeat_n(LEADER, leader_count));
    out
}
