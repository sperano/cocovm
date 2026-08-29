//! WAV audio export/import round trips for the cassette deck: encoding a
//! decoded tape as 8-bit PCM audio and decoding it back (including a
//! polarity-inverted and a widened-to-16-bit variant), plus the decoder's
//! rejection of malformed input. Split out of `cassette.rs` (which keeps the
//! playback/motor-gating/ROM end-to-end coverage) to stay under the
//! project's file-size guideline.

use coco_core::cassette::test_support::tape_block;
use coco_core::cassette_wav;

/// Leader/sync/framing bytes (Service Manual §5.10, `cassette-verified-facts`).
const LEADER: u8 = 0x55;

/// NTSC CPU clock (28.636363 MHz crystal / 32, MAME `coco3.cpp`): mirrors
/// the crate's own `CPU_HZ` / the value `coco_core::Machine::cpu_hz()`
/// returns — the same clock `Cassette::tick` and the WAV synth/decoder
/// measure tape bit periods against.
const CPU_HZ: f64 = 894_886.0;

/// Byte offset of PCM sample data in a WAV file built with the standard
/// 44-byte RIFF/WAVE header with `fmt ` and `data` chunks (no extra chunks).
const WAV_HEADER_LEN: usize = 44;

fn sample_tape() -> Vec<u8> {
    let mut tape = vec![LEADER; 16];
    tape.extend(tape_block(0x00, b"X       \x00\x00\x01\x3F\x00\x3F\x00"));
    tape.extend(vec![LEADER; 16]);
    tape.extend(tape_block(0x01, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x55]));
    tape.extend(tape_block(0xFF, &[]));
    tape
}

#[test]
fn wav_round_trip_preserves_the_tape_bytes() {
    let tape = sample_tape();
    let wav = cassette_wav::synthesize_wav(&tape, CPU_HZ);
    let decoded = cassette_wav::decode_wav(&wav, CPU_HZ).expect("a synthesized WAV must decode");
    assert_eq!(decoded, tape);
}

#[test]
fn wav_round_trip_survives_inverted_polarity() {
    let tape = sample_tape();
    let mut wav = cassette_wav::synthesize_wav(&tape, CPU_HZ);
    for sample in &mut wav[WAV_HEADER_LEN..] {
        *sample = 255 - *sample;
    }
    let decoded =
        cassette_wav::decode_wav(&wav, CPU_HZ).expect("a polarity-inverted WAV must still decode");
    assert_eq!(
        decoded, tape,
        "auto-polarity-detection must recover the original bytes"
    );
}

/// Minimal 16-bit mono PCM WAV builder for the following 16-bit round-trip test.
/// `cassette_wav`'s own header writer is private (it only ever emits 8-bit
/// audio) — this is a test-local helper exercising only the public
/// `decode_wav` surface with a hand-built 16-bit file.
fn build_16bit_wav(samples: &[i16]) -> Vec<u8> {
    const SAMPLE_RATE_HZ: u32 = 44_100;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;
    const PCM_FORMAT_TAG: u16 = 1;
    const FMT_CHUNK_LEN: u32 = 16;

    let byte_rate = SAMPLE_RATE_HZ * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
    let data_len = (samples.len() * 2) as u32;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(WAV_HEADER_LEN + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&FMT_CHUNK_LEN.to_le_bytes());
    out.extend_from_slice(&PCM_FORMAT_TAG.to_le_bytes());
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[test]
fn wav_round_trip_via_16_bit_pcm() {
    let tape = sample_tape();
    let wav8 = cassette_wav::synthesize_wav(&tape, CPU_HZ);
    let samples8 = &wav8[WAV_HEADER_LEN..];

    // Widen each 8-bit unsigned sample to the equivalent 16-bit signed one
    // (same waveform, different container).
    let samples16: Vec<i16> = samples8
        .iter()
        .map(|&s| (i16::from(s) - 128) * 256)
        .collect();
    let wav16 = build_16bit_wav(&samples16);

    let decoded16 = cassette_wav::decode_wav(&wav16, CPU_HZ).expect("a 16-bit WAV must decode");
    let decoded8 = cassette_wav::decode_wav(&wav8, CPU_HZ).expect("the 8-bit WAV must decode");
    assert_eq!(decoded16, decoded8);
}

#[test]
fn wav_decode_rejects_truncated_header() {
    let wav = cassette_wav::synthesize_wav(&sample_tape(), CPU_HZ);
    let truncated = &wav[..20];
    assert!(
        cassette_wav::decode_wav(truncated, CPU_HZ).is_err(),
        "a truncated header must error, not panic"
    );
}

#[test]
fn wav_decode_rejects_non_pcm_format_tag() {
    /// A common non-PCM tag that the decoder must reject.
    const IEEE_FLOAT_FORMAT_TAG: u16 = 3;
    let mut wav = cassette_wav::synthesize_wav(&sample_tape(), CPU_HZ);
    // The format tag is the 'fmt ' chunk body's first field, right after the
    // 8-byte "RIFF"+size, 4-byte "WAVE", and 8-byte "fmt "+size headers.
    const FORMAT_TAG_OFFSET: usize = 8 + 4 + 8;
    wav[FORMAT_TAG_OFFSET..FORMAT_TAG_OFFSET + 2]
        .copy_from_slice(&IEEE_FLOAT_FORMAT_TAG.to_le_bytes());
    assert!(
        matches!(
            cassette_wav::decode_wav(&wav, CPU_HZ),
            Err(cassette_wav::WAVError::UnsupportedFormatTag(
                IEEE_FLOAT_FORMAT_TAG
            ))
        ),
        "a non-PCM format tag must error, not panic or silently misparse"
    );
}
