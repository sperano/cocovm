//! Phase 6 acceptance test 4: cassette CSAVE/CLOAD (`docs/coco12-plan.md`)
//!
//! The cassette deck (`Cassette`, `bus.rs`'s PIA1 record/playback wiring) is
//! entirely machine-neutral — confirmed by inspection: `bus.rs::sam_io_write`'s
//! PIA1 branch feeds `Cassette::record_dac` exactly like the GIME path's
//! `io_write` does, and `Machine::pia1_pa_pins`'s cassette-input bit doesn't
//! consult `self.config.variant` at all. This is a regression guard for that
//! wiring on the plain-SAM bus path, mirroring `tests/cassette.rs`'s
//! `csave_rewind_cload_round_trips_a_basic_program` almost line-for-line, with
//! a CoCo 2 boot in place of the CoCo 3 one.

use coco_core::Machine;

use super::common::{boot_machine, boot_to_prompt, run_fields, screen_contains, screen_dump, type_line, SETTLE_FIELDS};

/// Tape leader/sync bytes (Service Manual §5.10) — matches `tests/cassette.rs`.
const LEADER: u8 = 0x55;
const SYNC: u8 = 0x3C;

/// Run until the cassette motor, having been on, stays off for a stretch
/// longer than any intra-operation pause, or until `max_fields` elapse.
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

/// Parse the framed blocks out of a decoded tape stream, asserting every
/// checksum — matches `tests/cassette.rs::parse_blocks`.
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
fn coco2_csave_rewind_cload_round_trips_a_basic_program() {
    const TAPE_OP_FIELDS: usize = 3000;
    const BLOCK_NAMEFILE: u8 = 0x00;
    const BLOCK_DATA: u8 = 0x01;
    const BLOCK_EOF: u8 = 0xFF;

    let Some((mut m, _bas_rom)) = boot_machine() else {
        return;
    };
    boot_to_prompt(&mut m);

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

    // Rewind finalizes the recording into the tape; check its structure.
    m.bus.cassette.rewind();
    let tape = m.bus.cassette.tape_bytes().to_vec();
    assert!(m.bus.cassette.dirty(), "a fresh recording must be dirty");
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
    assert!(
        !screen_contains(&mut m, "ERROR"),
        "CLOAD errored:\n{}",
        screen_dump(&mut m)
    );

    type_line(&mut m, "RUN");
    run_fields(&mut m, SETTLE_FIELDS);
    assert!(
        screen_contains(&mut m, "HI"),
        "the round-tripped program must print HI:\n{}",
        screen_dump(&mut m)
    );
}
