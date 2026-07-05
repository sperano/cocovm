//! Cassette deck coverage: playback/demodulator symmetry (no ROM needed),
//! motor gating, and an end-to-end CSAVE → Rewind → CLOAD → RUN round trip
//! against the real `roms/coco3.rom`.

use std::path::PathBuf;

use coco_core::cassette::{demodulate, Cassette, Transition};
use coco_core::{cassette_wav, Machine, MachineConfig};
use mc6809::Bus;

/// Leader/sync/framing bytes (Service Manual §5.10, `cassette-verified-facts`).
const LEADER: u8 = 0x55;
const SYNC: u8 = 0x3C;

/// One framed tape block: leader, sync, type, length, payload, checksum
/// (sum of type + length + payload), trailer.
fn tape_block(block_type: u8, payload: &[u8]) -> Vec<u8> {
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

// ============================================================================
// Playback → demodulator symmetry and motor gating (no ROM required)
// ============================================================================

/// Play a tape through the deck's own synthesizer, capture the squared PA0
/// signal as DAC-style transitions, and demodulate them back to bytes: the
/// full modulation path must be its own inverse.
#[test]
fn playback_waveform_demodulates_back_to_the_same_bytes() {
    /// Coarse instruction-sized tick, deliberately not a divisor of either
    /// bit period so phase error accumulates if the deck mishandles it.
    const TICK_CYCLES: u32 = 7;

    let mut tape = vec![LEADER; 16];
    tape.extend(tape_block(0x00, b"X       \x00\x00\x01\x3F\x00\x3F\x00"));
    tape.extend(vec![LEADER; 16]);
    tape.extend(tape_block(0x01, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x55]));
    tape.extend(tape_block(0xFF, &[]));

    let mut deck = Cassette::new();
    deck.insert_tape(tape.clone());

    // Sample the squared output into synthetic full-swing DAC transitions.
    // PA0 carries the SALT-inverted rendering of the tape signal, while the
    // demodulator consumes the DAC (record-side) domain — so map PA0 low to
    // DAC high. Skip the motor spin-up (tape not yet rolling, line idle).
    let mut capture: Vec<Transition> = Vec::new();
    let mut clock = 0u64;
    let mut last = None;
    // Burn through the motor spin-up (~0.5 s = 524288 cycles) in one gulp;
    // the tape holds still and the line idles until it drains.
    deck.tick(600_000, true);
    assert_eq!(deck.position().0, 0, "tape must hold still through spin-up");
    while deck.playing() {
        deck.tick(TICK_CYCLES, true);
        clock += u64::from(TICK_CYCLES);
        let level = if deck.input_bit() { 0 } else { 63 };
        if last != Some(level) {
            capture.push(Transition { level, cycle: clock });
            last = Some(level);
        }
    }

    assert_eq!(demodulate(&capture), tape);
}

#[test]
fn motor_off_freezes_the_tape_and_records_nothing() {
    let mut deck = Cassette::new();
    deck.insert_tape(vec![LEADER; 8]);
    deck.tick(10_000, false);
    assert_eq!(deck.position().0, 0, "tape must not move with the motor off");
    assert!(deck.input_bit(), "input idles high with the motor off");

    deck.record_dac(63, false);
    deck.record_dac(0, false);
    assert!(deck.capture().is_empty(), "nothing records with the motor off");
}

// ============================================================================
// WAV audio export/import round trips
// ============================================================================

/// NTSC CPU clock (28.636363 MHz crystal / 32, MAME `coco3.cpp`): mirrors
/// the private `CPU_HZ` in `coco-core/src/lib.rs` / the value
/// `coco_core::Machine::cpu_hz()` returns — the same clock `Cassette::tick`
/// and the WAV synth/decoder measure tape bit periods against.
const CPU_HZ: f64 = 894_886.0;

/// Byte offset of PCM sample data in a WAV file built with the standard
/// 44-byte RIFF/WAVE/`fmt `/`data` header layout (no extra chunks).
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
    assert_eq!(decoded, tape, "auto-polarity-detection must recover the original bytes");
}

/// Minimal 16-bit mono PCM WAV builder for the 16-bit round-trip test below.
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
    let samples16: Vec<i16> = samples8.iter().map(|&s| (i16::from(s) - 128) * 256).collect();
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
    /// WAVE_FORMAT_IEEE_FLOAT: a real, common non-PCM tag this must reject.
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
            Err(cassette_wav::WavError::UnsupportedFormatTag(IEEE_FLOAT_FORMAT_TAG))
        ),
        "a non-PCM format tag must error, not panic or silently misparse"
    );
}

// ============================================================================
// End-to-end against the real ROM
// ============================================================================

fn try_load_rom(name: &str) -> Option<Box<[u8]>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms")
        .join(name);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

fn screen_row(m: &mut Machine, row: u16) -> String {
    (0..32)
        .map(|c| {
            let code = m.bus.read(0x0400 + row * 32 + c) & 0x3F;
            if code < 0x20 { (b'@' + code) as char } else { (b' ' + (code - 0x20)) as char }
        })
        .collect()
}

fn screen_contains(m: &mut Machine, needle: &str) -> bool {
    (0..16).any(|r| screen_row(m, r).contains(needle))
}

fn screen_dump(m: &mut Machine) -> String {
    (0..16).map(|r| screen_row(m, r)).collect::<Vec<_>>().join("\n")
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn type_line(m: &mut Machine, s: &str) {
    for c in s.chars().chain(std::iter::once('\r')) {
        tap_char(m, c);
    }
}

/// Run until the cassette motor, having been on, stays off for a stretch
/// longer than any intra-operation pause (CSAVE's namefile→data gap is
/// ~0.5 s), or until `max_fields` elapse.
fn run_until_motor_idle(m: &mut Machine, max_fields: usize) {
    /// 1.5 s of motor-off at 60 fields/s — longer than any mid-tape gap.
    const IDLE_FIELDS: usize = 90;
    let mut seen_on = false;
    let mut off_streak = 0;
    for _ in 0..max_fields {
        m.run_field();
        if m.bus.pia1.a.c2_output() {
            seen_on = true;
            off_streak = 0;
        } else if seen_on {
            off_streak += 1;
            if off_streak >= IDLE_FIELDS {
                return;
            }
        }
    }
}

/// Parse the framed blocks out of a decoded tape stream: skip leader bytes,
/// then expect sync/type/len/payload/checksum/trailer. Returns
/// (type, payload) pairs and asserts every checksum.
fn parse_blocks(tape: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < tape.len() {
        if tape[i] == LEADER {
            i += 1;
            continue;
        }
        assert_eq!(tape[i], SYNC, "expected sync at offset {i}, got ${:02X}", tape[i]);
        let block_type = tape[i + 1];
        let len = usize::from(tape[i + 2]);
        let payload = tape[i + 3..i + 3 + len].to_vec();
        let checksum = tape[i + 3 + len];
        let expected = payload
            .iter()
            .fold((block_type).wrapping_add(len as u8), |acc, &b| acc.wrapping_add(b));
        assert_eq!(checksum, expected, "bad checksum in block type ${block_type:02X}");
        i += 3 + len + 1; // sync consumed through checksum; trailer is a LEADER
        blocks.push((block_type, payload));
    }
    blocks
}

#[test]
fn csave_rewind_cload_round_trips_a_basic_program() {
    const BOOT_FIELDS: usize = 300;
    const TAPE_OP_FIELDS: usize = 3000;
    /// Tape block types (Service Manual §5.10).
    const BLOCK_NAMEFILE: u8 = 0x00;
    const BLOCK_DATA: u8 = 0x01;
    const BLOCK_EOF: u8 = 0xFF;

    let Some(rom) = try_load_rom("coco3.rom") else {
        eprintln!("skipping csave_rewind_cload_round_trips_a_basic_program: roms/ not present");
        return;
    };
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }

    // Blank tape in the deck, record a program.
    m.bus.cassette.insert_tape(Vec::new());
    type_line(&mut m, "10 PRINT \"HI\"");
    type_line(&mut m, "CSAVE\"X\"");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(screen_contains(&mut m, "OK"), "CSAVE never finished:\n{}", screen_dump(&mut m));

    // Rewind finalizes the recording into the tape; check its structure.
    m.bus.cassette.rewind();
    let tape = m.bus.cassette.tape_bytes().to_vec();
    assert!(m.bus.cassette.dirty(), "a fresh recording must be dirty");
    let blocks = parse_blocks(&tape);
    assert_eq!(blocks[0].0, BLOCK_NAMEFILE);
    assert_eq!(&blocks[0].1[..8], b"X       ", "namefile name");
    assert_eq!(blocks[0].1.len(), 15, "namefile payload is 15 bytes");
    assert!(
        blocks[1..blocks.len() - 1].iter().all(|(t, _)| *t == BLOCK_DATA),
        "middle blocks are data blocks"
    );
    assert_eq!(blocks.last().unwrap().0, BLOCK_EOF);

    // Wipe BASIC's program, load it back from the tape, and run it.
    type_line(&mut m, "NEW");
    m.bus.cassette.rewind();
    type_line(&mut m, "CLOAD");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CLOAD never finished:\n{}",
        screen_dump(&mut m)
    );
    // (The blinking cursor masks to '?' in screen_row, so match "ERROR",
    // not '?'.)
    assert!(!screen_contains(&mut m, "ERROR"), "CLOAD errored:\n{}", screen_dump(&mut m));

    type_line(&mut m, "RUN");
    for _ in 0..30 {
        m.run_field();
    }
    assert!(
        screen_contains(&mut m, "HI"),
        "the round-tripped program must print HI:\n{}",
        screen_dump(&mut m)
    );
}
