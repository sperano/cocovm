//! Cassette deck coverage: playback/demodulator symmetry (no ROM needed),
//! motor gating, and an end-to-end CSAVE → Rewind → CLOAD → RUN round trip
//! against the real `roms/coco3.rom`. WAV audio export/import round trips
//! live in the sibling `cassette_wav.rs` (split out to stay under the
//! project's file-size guideline).

use std::path::PathBuf;

use coco_core::cassette::test_support::{SPINUP_BURN_CYCLES, record_bytes_fsk, tape_block};
use coco_core::cassette::{Cassette, RECORD_IDLE_FINALIZE_CYCLES, Transition, demodulate};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

/// Leader/sync/framing bytes (Service Manual §5.10, `cassette-verified-facts`).
const LEADER: u8 = 0x55;
const SYNC: u8 = 0x3C;

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
    // Burn through the motor spin-up in one gulp; the tape holds still and
    // the line idles until it drains.
    deck.tick(SPINUP_BURN_CYCLES, true);
    assert_eq!(deck.position().0, 0, "tape must hold still through spin-up");
    while deck.playing() {
        deck.tick(TICK_CYCLES, true);
        clock += u64::from(TICK_CYCLES);
        let level = if deck.input_bit() { 0 } else { 63 };
        if last != Some(level) {
            capture.push(Transition {
                level,
                cycle: clock,
            });
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
    assert_eq!(
        deck.position().0,
        0,
        "tape must not move with the motor off"
    );
    assert!(deck.input_bit(), "input idles high with the motor off");

    deck.record_dac(63, false);
    deck.record_dac(0, false);
    assert!(
        deck.capture().is_empty(),
        "nothing records with the motor off"
    );
}

/// While a recording is in flight, `position()` reports the live estimate
/// of bytes recorded so far (one tone cycle per bit, counted off the DAC
/// midpoint crossings) instead of the parked playback position — the status
/// bar's counter moves during CSAVE, not just CLOAD.
#[test]
fn position_tracks_a_recording_in_flight() {
    let mut deck = Cassette::new();
    deck.insert_tape(Vec::new());
    assert_eq!(deck.position(), (0, 0));

    // 16 full-swing tone cycles into the record tap: 16 bits = 2 bytes.
    for _ in 0..16 {
        deck.record_dac(63, true);
        deck.record_dac(0, true);
    }
    assert_eq!(
        deck.position(),
        (2, 2),
        "the counter must move with the bytes recorded, before any finalize"
    );

    // Finalizing discards a sync-less capture (stray DAC noise) and the
    // counter falls back to the unchanged playback position.
    deck.finalize_recording();
    assert_eq!(deck.position(), (0, 0));
}

/// A real deck records starting at the head's current position, splicing
/// the new recording into the tape rather than replacing the whole reel.
/// CSAVE "T1" -> rewind -> CLOAD (head parks after T1) -> CSAVE "T2" must
/// leave both files on the tape, back to back — not wipe T1.
#[test]
fn recording_splices_at_the_head_position() {
    /// Coarse instruction-sized tick for draining playback (see
    /// `playback_waveform_demodulates_back_to_the_same_bytes`).
    const TICK_CYCLES: u32 = 7;

    let mut tape_a = vec![LEADER; 8];
    tape_a.extend(tape_block(0x01, b"FILE-A"));
    let mut tape_b = vec![LEADER; 8];
    tape_b.extend(tape_block(0x01, b"FILE-B"));

    let mut deck = Cassette::new();
    deck.insert_tape(tape_a.clone());

    // Burn spin-up, then drain playback to the end so the head parks right
    // after file A.
    deck.tick(SPINUP_BURN_CYCLES, true);
    while deck.playing() {
        deck.tick(TICK_CYCLES, true);
    }
    assert_eq!(deck.position().0, tape_a.len(), "head parked after file A");

    // Record file B starting from the parked head position.
    record_bytes_fsk(&mut deck, &tape_b);
    assert!(
        deck.position().0 >= tape_a.len(),
        "the live estimate must count from the splice anchor, not from 0"
    );

    deck.finalize_recording();
    let expected = [tape_a.clone(), tape_b.clone()].concat();
    assert_eq!(
        deck.tape_bytes(),
        expected.as_slice(),
        "the tape must hold both files back to back"
    );
    assert!(deck.dirty());
    let full_len = tape_a.len() + tape_b.len();
    assert_eq!(
        deck.position(),
        (full_len, full_len),
        "the head parks at the end of the spliced-in stretch"
    );

    // Recording from a rewound (anchor 0) head truncates and overwrites
    // from the top, matching the old whole-tape-replace behaviour.
    let mut tape_c = vec![LEADER; 8];
    tape_c.extend(tape_block(0x01, b"FILE-C"));
    deck.rewind();
    record_bytes_fsk(&mut deck, &tape_c);
    deck.finalize_recording();
    assert_eq!(
        deck.tape_bytes(),
        tape_c.as_slice(),
        "recording from a rewound head overwrites the whole tape"
    );
}

/// A recording left in flight when the motor stops must land on the tape
/// by itself once the motor has been idle long enough — no rewind/eject
/// required — but a pause no longer than CSAVE's own namefile->data gap
/// (~0.5 s) must NOT trip it early.
#[test]
fn recording_finalizes_itself_after_motor_idle() {
    let mut deck = Cassette::new();
    deck.insert_tape(Vec::new());
    deck.tick(SPINUP_BURN_CYCLES, true); // burn spin-up

    let mut block = vec![LEADER; 16];
    block.extend(tape_block(0x01, b"HI"));
    record_bytes_fsk(&mut deck, &block);
    assert!(
        !deck.capture().is_empty(),
        "the capture must be pending after recording"
    );

    // Half the threshold (~1 s): below it, so a CSAVE-style intra-operation
    // gap must survive untouched.
    const HALF_THRESHOLD: u32 = (RECORD_IDLE_FINALIZE_CYCLES / 2) as u32;
    deck.tick(HALF_THRESHOLD, false);
    assert!(
        !deck.capture().is_empty(),
        "a sub-threshold motor-off gap must not finalize the in-flight recording"
    );
    assert!(
        !deck.take_recording_landed(),
        "must not report a finalize before the threshold"
    );

    // Cross the threshold.
    deck.tick(
        RECORD_IDLE_FINALIZE_CYCLES as u32 - HALF_THRESHOLD + 1,
        false,
    );
    assert!(
        deck.capture().is_empty(),
        "auto-finalize must consume the pending capture"
    );
    assert_eq!(
        deck.tape_bytes(),
        block.as_slice(),
        "the decoded bytes must have landed on the tape"
    );
    assert!(deck.dirty());
    assert!(
        deck.take_recording_landed(),
        "a landed finalize must report true once"
    );
    assert!(
        !deck.take_recording_landed(),
        "the flag must clear after being taken"
    );
}

/// A capture that never contains a valid sync (stray DAC noise, not a real
/// recording) idling out must be discarded exactly like an explicit
/// finalize discards one — tape unchanged, and no finalize event reported
/// (nothing landed).
#[test]
fn idle_finalize_discards_a_syncless_capture_without_the_flag() {
    let mut deck = Cassette::new();
    let original = vec![LEADER; 4];
    deck.insert_tape(original.clone());

    // A few raw DAC transitions with no leader/sync framing (mirrors
    // `position_tracks_a_recording_in_flight`'s capture).
    for _ in 0..4 {
        deck.record_dac(63, true);
        deck.record_dac(0, true);
    }
    assert!(!deck.capture().is_empty());

    deck.tick(RECORD_IDLE_FINALIZE_CYCLES as u32 + 1, false);
    assert_eq!(
        deck.tape_bytes(),
        original.as_slice(),
        "a sync-less capture idling out must not alter the tape"
    );
    assert!(
        !deck.take_recording_landed(),
        "a discarded capture must not report a landed finalize"
    );
}

/// Byte-granular seek: the UI's "seek to byte" control moves the head
/// straight to a position, clamped to the tape's end.
#[test]
fn seek_moves_the_head_and_clamps_to_the_tape_end() {
    let mut deck = Cassette::new();
    deck.insert_tape(vec![LEADER; 100]);

    deck.seek(50);
    assert_eq!(deck.position().0, 50);

    deck.seek(usize::MAX);
    assert_eq!(
        deck.position().0,
        100,
        "seeking past the end must clamp to the tape length"
    );
}

/// Seeking while a recording is in flight must finalize it first (splicing
/// it into the tape), exactly like rewind — otherwise the in-flight capture
/// would be silently dropped by the head jumping out from under it.
#[test]
fn seek_finalizes_a_pending_recording_first() {
    let mut deck = Cassette::new();
    deck.insert_tape(Vec::new());
    deck.tick(SPINUP_BURN_CYCLES, true); // burn spin-up

    let mut block = vec![LEADER; 16];
    block.extend(tape_block(0x01, b"HI"));
    record_bytes_fsk(&mut deck, &block);
    assert!(
        !deck.capture().is_empty(),
        "the capture must be pending before the seek"
    );

    deck.seek(0);
    assert!(
        deck.capture().is_empty(),
        "seek must finalize the in-flight recording"
    );
    assert!(deck.dirty());
    assert_eq!(
        deck.tape_bytes(),
        block.as_slice(),
        "the finalized recording must have been spliced onto the tape"
    );
    assert_eq!(
        deck.position().0,
        0,
        "the head must land at the seek target"
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
            if code < 0x20 {
                (b'@' + code) as char
            } else {
                (b' ' + (code - 0x20)) as char
            }
        })
        .collect()
}

fn screen_contains(m: &mut Machine, needle: &str) -> bool {
    (0..16).any(|r| screen_row(m, r).contains(needle))
}

fn screen_dump(m: &mut Machine) -> String {
    (0..16)
        .map(|r| screen_row(m, r))
        .collect::<Vec<_>>()
        .join("\n")
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
        assert_eq!(
            tape[i], SYNC,
            "expected sync at offset {i}, got ${:02X}",
            tape[i]
        );
        let block_type = tape[i + 1];
        let len = usize::from(tape[i + 2]);
        let payload = tape[i + 3..i + 3 + len].to_vec();
        let checksum = tape[i + 3 + len];
        let expected = payload
            .iter()
            .fold((block_type).wrapping_add(len as u8), |acc, &b| {
                acc.wrapping_add(b)
            });
        assert_eq!(
            checksum, expected,
            "bad checksum in block type ${block_type:02X}"
        );
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
    assert!(
        screen_contains(&mut m, "OK"),
        "CSAVE never finished:\n{}",
        screen_dump(&mut m)
    );

    // The counter tracked the recording live: with CSAVE done but the
    // recording not yet finalized, the reported position is the record
    // estimate, not the parked playback position.
    let (recorded, live_len) = m.bus.cassette.position();
    assert!(recorded > 0, "the counter must have moved during CSAVE");
    assert_eq!(live_len, recorded, "a growing recording is its own length");

    // Rewind finalizes the recording into the tape; check its structure.
    m.bus.cassette.rewind();
    let tape = m.bus.cassette.tape_bytes().to_vec();
    assert!(m.bus.cassette.dirty(), "a fresh recording must be dirty");
    // The live estimate counts raw tone cycles; the decoder additionally
    // spends a few bits re-hunting byte alignment across the namefile→data
    // motor gap, so the two lengths agree only to within a few bytes.
    const RECORD_ESTIMATE_SLACK: usize = 16;
    assert!(
        recorded.abs_diff(tape.len()) <= RECORD_ESTIMATE_SLACK,
        "live record estimate ({recorded}) must approximate the decoded tape \
         length ({})",
        tape.len()
    );
    let blocks = parse_blocks(&tape);
    assert_eq!(blocks[0].0, BLOCK_NAMEFILE);
    assert_eq!(&blocks[0].1[..8], b"X       ", "namefile name");
    assert_eq!(blocks[0].1.len(), 15, "namefile payload is 15 bytes");
    assert!(
        blocks[1..blocks.len() - 1]
            .iter()
            .all(|(t, _)| *t == BLOCK_DATA),
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
    assert!(
        !screen_contains(&mut m, "ERROR"),
        "CLOAD errored:\n{}",
        screen_dump(&mut m)
    );

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

/// CSAVE "A" -> rewind -> CLOAD (parks the head after A's blocks) -> CSAVE
/// "B" must splice file B onto the tape after file A, not wipe A — then
/// both files must CLOAD back in order.
#[test]
fn csave_cload_csave_builds_a_two_file_tape() {
    const BOOT_FIELDS: usize = 300;
    const TAPE_OP_FIELDS: usize = 3000;
    /// Tape block types (Service Manual §5.10).
    const BLOCK_NAMEFILE: u8 = 0x00;
    const BLOCK_EOF: u8 = 0xFF;

    let Some(rom) = try_load_rom("coco3.rom") else {
        eprintln!("skipping csave_cload_csave_builds_a_two_file_tape: roms/ not present");
        return;
    };
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }

    // Blank tape, save file A.
    m.bus.cassette.insert_tape(Vec::new());
    type_line(&mut m, "10 PRINT \"HI\"");
    type_line(&mut m, "CSAVE\"A\"");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CSAVE\"A\" never finished:\n{}",
        screen_dump(&mut m)
    );

    // Rewind (finalizes A's recording), load it back — this parks the head
    // right after A's blocks.
    m.bus.cassette.rewind();
    type_line(&mut m, "NEW");
    type_line(&mut m, "CLOAD");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CLOAD of file A never finished:\n{}",
        screen_dump(&mut m)
    );

    // Redefine the program and save it as file B, from the parked head.
    type_line(&mut m, "10 PRINT \"BYE\"");
    type_line(&mut m, "CSAVE\"B\"");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        screen_contains(&mut m, "OK"),
        "CSAVE\"B\" never finished:\n{}",
        screen_dump(&mut m)
    );

    // Rewind finalizes the splice of B onto the tail. Both files must be on
    // the tape, back to back, in order.
    m.bus.cassette.rewind();
    let tape = m.bus.cassette.tape_bytes().to_vec();
    let blocks = parse_blocks(&tape);
    let namefiles: Vec<&[u8]> = blocks
        .iter()
        .filter(|(t, _)| *t == BLOCK_NAMEFILE)
        .map(|(_, payload)| &payload[..8])
        .collect();
    assert_eq!(
        namefiles,
        vec![b"A       ".as_slice(), b"B       ".as_slice()],
        "both files must be present, in order:\n{}",
        screen_dump(&mut m)
    );
    // Each namefile block is eventually followed by an EOF block somewhere
    // later in the block list.
    for (i, (block_type, _)) in blocks.iter().enumerate() {
        if *block_type == BLOCK_NAMEFILE {
            assert!(
                blocks[i + 1..].iter().any(|(t, _)| *t == BLOCK_EOF),
                "namefile block at index {i} has no trailing EOF block"
            );
        }
    }

    // Both files must CLOAD back, in order — the second CLOAD only works if
    // the head parked after A's data rather than the tape being wiped down
    // to just B.
    type_line(&mut m, "NEW");
    type_line(&mut m, "CLOAD");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        !screen_contains(&mut m, "ERROR"),
        "first CLOAD (file A) errored:\n{}",
        screen_dump(&mut m)
    );
    type_line(&mut m, "CLOAD");
    run_until_motor_idle(&mut m, TAPE_OP_FIELDS);
    assert!(
        !screen_contains(&mut m, "ERROR"),
        "second CLOAD (file B) errored:\n{}",
        screen_dump(&mut m)
    );

    type_line(&mut m, "RUN");
    for _ in 0..30 {
        m.run_field();
    }
    assert!(
        screen_contains(&mut m, "BYE"),
        "the second CLOAD must have loaded file B's redefined program:\n{}",
        screen_dump(&mut m)
    );
}
