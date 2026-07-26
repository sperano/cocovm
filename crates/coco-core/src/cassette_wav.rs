//! WAV audio export/import for the cassette subsystem ([`crate::cassette`]):
//! turns the deck's decoded .cas byte stream into real tape audio, and real
//! tape audio back into decoded bytes — so a tape can be played into or
//! recorded from actual cassette hardware, or exchanged with tools that only
//! speak audio, not the .cas convention.
//!
//! This module owns the *audio* domain only. The bit-timing facts it needs
//! (tone periods, LSB-first bit order, the sync byte) are measured and
//! documented in `cassette.rs` and reused here via `pub(crate)` items rather
//! than duplicated: [`ZERO_BIT_PERIOD`], [`ONE_BIT_PERIOD`], [`SYNC`].

use crate::cassette::{ONE_BIT_PERIOD, SYNC, Transition, ZERO_BIT_PERIOD, demodulate};

// ---- Shared constants ------------------------------------------------------

/// WAVE_FORMAT_PCM: the only `fmt` tag this module writes or accepts.
const PCM_FORMAT_TAG: u16 = 1;

/// WAV sample rate used when synthesizing tape audio: a standard rate broadly
/// compatible with sound editors, tape-deck line inputs, and other CoCo
/// emulators' WAV export. Not a CoCo hardware fact — an arbitrary (if
/// conventional) choice for compatibility.
const WAV_SAMPLE_RATE_HZ: u32 = 44_100;

/// 8-bit unsigned PCM's silence/center level (the format's zero-signal
/// value).
const WAV_MIDPOINT: u8 = 128;

/// Sine amplitude around [`WAV_MIDPOINT`]: the largest that keeps both the
/// peak (128+127=255) and trough (128-127=1) inside `u8`.
const WAV_AMPLITUDE: f64 = 127.0;

/// Silence prepended before the tape audio starts: gives a real cassette
/// deck's motor and AGC time to spin up and settle before data appears — an
/// engineering convention (mirrors MAME's `.cas`-to-audio cassette loader
/// lead-in), not a hardware timing fact.
const WAV_LEAD_IN_SECS: f64 = 1.0;

/// Silence appended after the tape audio ends, same convention as
/// [`WAV_LEAD_IN_SECS`].
const WAV_LEAD_OUT_SECS: f64 = 0.25;

// ---- Export: decoded tape bytes -> WAV -------------------------------------

/// Re-synthesize `tape` (a decoded .cas byte stream, LSB-first per
/// [`crate::cassette::Cassette::input_bit`]) as audio: one full sine cycle
/// per bit, at [`ONE_BIT_PERIOD`]/[`ZERO_BIT_PERIOD`] converted from CPU
/// cycles to seconds via `cpu_hz` ([`crate::Machine::cpu_hz`]).
///
/// The sine starts rising from the midpoint (phase 0), matching the ROM's
/// own DAC sine table directly — unlike
/// [`crate::cassette::Cassette::input_bit`]'s squared PA0 output, this is
/// *not* inverted. That inversion is specific to the SALT chip's analog
/// record→play path reproducing a squared rendition of the tape signal on
/// PA0; this function instead synthesizes what a real cassette deck's line
/// input jack expects to receive: the direct sine the DAC wrote.
pub fn synthesize_wav(tape: &[u8], cpu_hz: f64) -> Vec<u8> {
    let mut samples = Vec::new();
    push_silence(&mut samples, WAV_LEAD_IN_SECS);

    for &byte in tape {
        for bit_index in 0..8u8 {
            let one = byte >> bit_index & 1 == 1; // LSB first
            let period_cycles = if one { ONE_BIT_PERIOD } else { ZERO_BIT_PERIOD };
            push_sine_cycle(&mut samples, period_cycles, cpu_hz);
        }
    }

    push_silence(&mut samples, WAV_LEAD_OUT_SECS);
    build_wav_bytes(&samples)
}

/// Append `secs` worth of constant-midpoint (silent) samples.
fn push_silence(samples: &mut Vec<u8>, secs: f64) {
    let count = (secs * f64::from(WAV_SAMPLE_RATE_HZ)).round() as usize;
    samples.extend(std::iter::repeat_n(WAV_MIDPOINT, count));
}

/// Append one full sine cycle representing a single tape bit's tone.
fn push_sine_cycle(samples: &mut Vec<u8>, period_cycles: u32, cpu_hz: f64) {
    let period_secs = f64::from(period_cycles) / cpu_hz;
    let cycle_samples = (period_secs * f64::from(WAV_SAMPLE_RATE_HZ)).round().max(1.0) as usize;
    for i in 0..cycle_samples {
        let phase = i as f64 / cycle_samples as f64;
        let value = f64::from(WAV_MIDPOINT) + WAV_AMPLITUDE * (std::f64::consts::TAU * phase).sin();
        // Float-to-int casts saturate (Rust since 1.45), so this can't
        // overflow even with float rounding at the extremes.
        samples.push(value.round() as u8);
    }
}

/// Hand-rolled 44-byte RIFF/WAVE header followed by the PCM sample data:
/// mono, 8-bit unsigned, [`WAV_SAMPLE_RATE_HZ`].
fn build_wav_bytes(samples: &[u8]) -> Vec<u8> {
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 8;
    const FMT_CHUNK_LEN: u32 = 16;

    let byte_rate = WAV_SAMPLE_RATE_HZ * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
    let data_len = samples.len() as u32;
    // RIFF chunk size excludes the leading "RIFF"+size(4) fields themselves:
    // 4 for "WAVE" + 8+FMT_CHUNK_LEN for the fmt chunk (header+body) +
    // 8+data_len for the data chunk (header+samples) = 36 + data_len.
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + samples.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&FMT_CHUNK_LEN.to_le_bytes());
    out.extend_from_slice(&PCM_FORMAT_TAG.to_le_bytes());
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&WAV_SAMPLE_RATE_HZ.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(samples);
    out
}

// ---- Import: WAV -> decoded tape bytes -------------------------------------

/// Error decoding a WAV file into cassette tape bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WavError {
    /// Fewer bytes than the minimal `RIFF`+size(4)+`WAVE` header.
    Truncated,
    /// Missing/mismatched `RIFF`/`WAVE` magic.
    NotWav,
    /// A chunk header claims a size that runs past the end of the buffer.
    ChunkOverrun { chunk_id: [u8; 4], offset: usize, size: usize, remaining: usize },
    /// No `fmt ` chunk found before the buffer ran out.
    MissingFmtChunk,
    /// `fmt ` chunk shorter than the minimal 16-byte PCM format body.
    FmtChunkTooShort,
    /// The `fmt ` chunk's format tag wasn't 1 (PCM) — e.g. 3 (IEEE float) or
    /// 6/7 (A-law/mu-law).
    UnsupportedFormatTag(u16),
    /// The `fmt ` chunk declared a bit depth other than 8 or 16.
    UnsupportedBitsPerSample(u16),
    /// The `fmt ` chunk declared zero channels.
    NoChannels,
    /// No `data` chunk found before the buffer ran out.
    MissingDataChunk,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavError::Truncated => write!(f, "WAV file is truncated (missing RIFF/WAVE header)"),
            WavError::NotWav => write!(f, "not a WAV file (missing RIFF/WAVE magic)"),
            WavError::ChunkOverrun { chunk_id, offset, size, remaining } => write!(
                f,
                "WAV chunk {:?} at offset {offset} claims {size} bytes but only {remaining} remain",
                String::from_utf8_lossy(chunk_id)
            ),
            WavError::MissingFmtChunk => write!(f, "WAV file has no 'fmt ' chunk"),
            WavError::FmtChunkTooShort => {
                write!(f, "WAV 'fmt ' chunk is shorter than the minimal 16-byte PCM format body")
            }
            WavError::UnsupportedFormatTag(tag) => {
                write!(f, "unsupported WAV format tag {tag} (only PCM/1 is supported)")
            }
            WavError::UnsupportedBitsPerSample(bits) => {
                write!(f, "unsupported WAV sample depth {bits}-bit (only 8-bit or 16-bit are supported)")
            }
            WavError::NoChannels => write!(f, "WAV 'fmt ' chunk declares zero channels"),
            WavError::MissingDataChunk => write!(f, "WAV file has no 'data' chunk"),
        }
    }
}

impl std::error::Error for WavError {}

/// Minimum bytes to hold `"RIFF"` + size(4) + `"WAVE"` before any chunk
/// walking can begin.
const RIFF_HEADER_LEN: usize = 12;
/// Chunk header: 4-byte id + 4-byte little-endian size.
const CHUNK_HEADER_LEN: usize = 8;
/// Minimal PCM `fmt ` chunk body: tag(2), channels(2), sample rate(4), byte
/// rate(4), block align(2), bits per sample(2).
const PCM_FMT_CHUNK_LEN: usize = 16;

/// The subset of the `fmt ` chunk this module cares about.
struct WavFmt {
    channels: u16,
    sample_rate_hz: u32,
    bits_per_sample: u16,
}

/// Walk a RIFF/WAVE file's chunks (not assuming `fmt ` immediately follows
/// the header — `LIST`/`INFO` chunks commonly come first) and return the
/// `fmt` info plus the `data` chunk's payload slice.
fn parse_wav_chunks(bytes: &[u8]) -> Result<(WavFmt, &[u8]), WavError> {
    if bytes.len() < RIFF_HEADER_LEN {
        return Err(WavError::Truncated);
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }

    let mut fmt: Option<WavFmt> = None;
    let mut data: Option<&[u8]> = None;
    let mut offset = RIFF_HEADER_LEN;
    while offset + CHUNK_HEADER_LEN <= bytes.len() {
        let (chunk_id, body, next_offset) = read_chunk(bytes, offset)?;
        if &chunk_id == b"fmt " {
            fmt = Some(parse_fmt_chunk(body)?);
        } else if &chunk_id == b"data" {
            data = Some(body);
        }
        offset = next_offset;
    }

    let fmt = fmt.ok_or(WavError::MissingFmtChunk)?;
    let data = data.ok_or(WavError::MissingDataChunk)?;
    Ok((fmt, data))
}

/// Read one chunk header (id + little-endian size) and its body at `offset`,
/// returning `(id, body, next_offset)` — `next_offset` already accounts for
/// RIFF's even-size chunk padding (the pad byte isn't part of the declared
/// size).
fn read_chunk(bytes: &[u8], offset: usize) -> Result<([u8; 4], &[u8], usize), WavError> {
    let chunk_id: [u8; 4] = bytes[offset..offset + 4].try_into().unwrap();
    let chunk_size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
    let body_start = offset + CHUNK_HEADER_LEN;
    let body_end = body_start.checked_add(chunk_size).filter(|&e| e <= bytes.len());
    let Some(body_end) = body_end else {
        return Err(WavError::ChunkOverrun {
            chunk_id,
            offset,
            size: chunk_size,
            remaining: bytes.len().saturating_sub(body_start),
        });
    };
    let body = &bytes[body_start..body_end];
    let next_offset = body_end + (chunk_size % 2);
    Ok((chunk_id, body, next_offset))
}

/// Parse a `fmt ` chunk body into [`WavFmt`], rejecting anything this module
/// doesn't decode (non-PCM, zero channels, unsupported sample depth).
fn parse_fmt_chunk(body: &[u8]) -> Result<WavFmt, WavError> {
    if body.len() < PCM_FMT_CHUNK_LEN {
        return Err(WavError::FmtChunkTooShort);
    }
    let format_tag = u16::from_le_bytes(body[0..2].try_into().unwrap());
    if format_tag != PCM_FORMAT_TAG {
        return Err(WavError::UnsupportedFormatTag(format_tag));
    }
    let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
    if channels == 0 {
        return Err(WavError::NoChannels);
    }
    let sample_rate_hz = u32::from_le_bytes(body[4..8].try_into().unwrap());
    let bits_per_sample = u16::from_le_bytes(body[14..16].try_into().unwrap());
    if bits_per_sample != 8 && bits_per_sample != 16 {
        return Err(WavError::UnsupportedBitsPerSample(bits_per_sample));
    }
    Ok(WavFmt { channels, sample_rate_hz, bits_per_sample })
}

/// Extract channel 0's samples as signed integers on a common scale,
/// regardless of the WAV's bit depth (8-bit unsigned or 16-bit signed).
/// Other channels are simply skipped (every Nth sample, N = channel count).
fn extract_mono_samples(data: &[u8], fmt: &WavFmt) -> Vec<i32> {
    let channels = usize::from(fmt.channels);
    let mut out = Vec::new();
    match fmt.bits_per_sample {
        8 => {
            for frame in data.chunks_exact(channels) {
                out.push(i32::from(frame[0]));
            }
        }
        16 => {
            for frame in data.chunks_exact(channels * 2) {
                let sample = i16::from_le_bytes([frame[0], frame[1]]);
                out.push(i32::from(sample));
            }
        }
        other => unreachable!("bits_per_sample {other} was validated to 8 or 16 in parse_wav_chunks"),
    }
    out
}

/// Hysteresis band half-width as a fraction of the signal's peak-to-peak
/// range, centered on the midpoint: real recordings carry noise near the
/// zero crossing, so without a deadband a slow drift around the midpoint
/// alone would produce spurious extra transitions. A tunable heuristic, not
/// a hardware fact.
const WAV_HYSTERESIS_FRACTION: f64 = 0.1;

/// Detect zero-crossings of the signal's own midpoint (with hysteresis
/// against noise) and emit a [`Transition`] only when the side changes — the
/// same sparse-transition-list domain [`demodulate`] consumes. `mid` and
/// `hysteresis` are computed from the capture's own min/max, not assumed
/// from bit depth, since real recordings rarely hit full scale.
///
/// `invert` negates each sample around `mid` (`2*mid - sample`) before
/// thresholding, per the spec's polarity-uncertainty workaround (mirroring
/// the phase-uncertainty lesson `cassette.rs` already documents for
/// playback — a recording can come in with either polarity and only trying
/// to demodulate both tells you which). This must be a real negation of the
/// compared value, not merely swapping which of [`Transition::level`]'s two
/// constants means "high": at an exact tie (`sample == mid`, e.g. the lead-
/// in/out silence sits exactly there) negation leaves the sample AT `mid`
/// in both polarities, so it must stay off both thresholds either way — a
/// post-hoc label swap instead flips that tied sample's apparent side,
/// which [`demodulate`]'s first-sample handling is sensitive to (it treats
/// a capture that opens on the high side specially).
fn capture_transitions(
    samples: &[i32],
    mid: f64,
    hysteresis: f64,
    invert: bool,
    cpu_hz: f64,
    wav_sample_rate_hz: u32,
) -> Vec<Transition> {
    const HIGH_LEVEL: u8 = 255;
    const LOW_LEVEL: u8 = 0;

    let rising_threshold = mid + hysteresis;
    let falling_threshold = mid - hysteresis;
    let signal = |raw: i32| -> f64 {
        let s = f64::from(raw);
        if invert { 2.0 * mid - s } else { s }
    };

    let mut state_high = signal(samples[0]) > mid;
    let mut out = Vec::new();
    out.push(Transition {
        level: if state_high { HIGH_LEVEL } else { LOW_LEVEL },
        cycle: sample_to_cycle(0, cpu_hz, wav_sample_rate_hz),
    });
    for (i, &raw) in samples.iter().enumerate().skip(1) {
        let s = signal(raw);
        if !state_high && s > rising_threshold {
            state_high = true;
            out.push(Transition {
                level: HIGH_LEVEL,
                cycle: sample_to_cycle(i, cpu_hz, wav_sample_rate_hz),
            });
        } else if state_high && s < falling_threshold {
            state_high = false;
            out.push(Transition {
                level: LOW_LEVEL,
                cycle: sample_to_cycle(i, cpu_hz, wav_sample_rate_hz),
            });
        }
    }
    out
}

/// Map a WAV sample index to the CPU-cycle clock [`demodulate`] expects
/// ([`Transition::cycle`]'s domain).
fn sample_to_cycle(sample_index: usize, cpu_hz: f64, wav_sample_rate_hz: u32) -> u64 {
    (sample_index as f64 * cpu_hz / f64::from(wav_sample_rate_hz)).round() as u64
}

/// Between two demodulated candidates from the two polarity guesses, prefer
/// the one containing the block [`SYNC`] byte (a much stronger signal that
/// the polarity/framing was right than raw length), falling back to the
/// longer decode when either both or neither contain it.
fn choose_best_decode(a: Vec<u8>, b: Vec<u8>) -> Vec<u8> {
    match (a.contains(&SYNC), b.contains(&SYNC)) {
        (true, false) => a,
        (false, true) => b,
        _ => {
            if a.len() >= b.len() { a } else { b }
        }
    }
}

/// Decode a WAV file back into a cassette tape's decoded byte stream.
///
/// Accepts 8-bit unsigned or 16-bit signed PCM, any channel count (channel 0
/// only) and any sample rate (read from the `fmt` chunk). Since a recording
/// can come in with either polarity (mirroring the phase-uncertainty
/// [`crate::cassette`] already documents for playback), the whole
/// crossing-detection + [`demodulate`] pipeline runs twice — once as read,
/// once inverted — and [`choose_best_decode`] keeps the better result.
///
/// `cpu_hz` is the machine's CPU clock ([`crate::Machine::cpu_hz`]), used to
/// convert WAV sample indices into the CPU-cycle domain `demodulate` expects.
pub fn decode_wav(bytes: &[u8], cpu_hz: f64) -> Result<Vec<u8>, WavError> {
    let (fmt, data) = parse_wav_chunks(bytes)?;
    let samples = extract_mono_samples(data, &fmt);
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let min = *samples.iter().min().unwrap();
    let max = *samples.iter().max().unwrap();
    let mid = f64::from(min + max) / 2.0;
    let peak_to_peak = f64::from(max - min);
    let hysteresis = peak_to_peak * WAV_HYSTERESIS_FRACTION / 2.0;

    let normal = capture_transitions(&samples, mid, hysteresis, false, cpu_hz, fmt.sample_rate_hz);
    let inverted = capture_transitions(&samples, mid, hysteresis, true, cpu_hz, fmt.sample_rate_hz);

    Ok(choose_best_decode(demodulate(&normal), demodulate(&inverted)))
}
