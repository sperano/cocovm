//! End-to-end regression for the bit-banger printer port (T2): boot the real
//! Super Extended Color BASIC ROM, type a short program,
//! `LLIST` it, and assert the decoded bytes reaching an in-memory
//! [`CaptureSink`] match the listing BASIC actually sent — proving the whole
//! path (PIA1 DDR/CRA setup, the ROM's bit-bang transmit loop, the BUSY
//! handshake, and `BitBanger`'s RX decoder) works against unmodified ROM
//! code, not only synthetic edge timings. Skips gracefully if `roms/` isn't
//! present, matching `tests/boot.rs`/`tests/fdc.rs`.

use coco_core::bitbanger::CaptureSink;
use coco_core::{Machine, MachineConfig};
use test_assets::rom::COCO3;

fn load_rom() -> Option<Box<[u8]>> {
    let path = test_assets::rom(COCO3);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

/// Hold key `pos` down for 3 fields, then release it for 3 more — long enough
/// for the keyboard-poll housekeeping IRQ (PIA0 CB1) to see it land and lift,
/// same shape as `tests/vhd_boot.rs`/`tests/fdc.rs`.
fn tap(m: &mut Machine, pos: (u8, u8)) {
    for _ in 0..3 {
        m.bus.keyboard.set(pos, true);
        m.run_field();
    }
    m.bus.keyboard.set(pos, false);
    for _ in 0..3 {
        m.run_field();
    }
}

fn tap_char(m: &mut Machine, c: char) {
    let (pos, shift) =
        coco_core::keyboard::char_key(c).unwrap_or_else(|| panic!("no key for {c:?}"));
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
    }
    tap(m, pos);
    if shift {
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
    }
}

fn type_str(m: &mut Machine, s: &str) {
    for c in s.chars() {
        tap_char(m, c);
    }
}

/// Count of rows that are exactly `OK` once trailing padding is trimmed —
/// `text_screen_lines` returns fixed-width (32-column) rows, so a bare
/// `contains("OK")` marker (as `tests/vhd_boot.rs`'s `wait_for` uses for
/// multi-character markers) can't tell "the prompt reappeared" from "the
/// prompt from before is still on screen"; counting exact-match rows and
/// waiting for the count to grow can.
fn ok_prompt_count(m: &mut Machine) -> usize {
    m.text_screen_lines()
        .iter()
        .filter(|line| line.trim_end() == "OK")
        .count()
}

/// Run fields in `POLL_FIELDS`-sized batches until the number of `OK` prompt
/// rows on screen exceeds `baseline`, panicking with the final screen after
/// `max_fields` (same shape as `tests/vhd_boot.rs`'s `wait_for`, adapted to a
/// count-based marker since `OK` can already be on screen before the action
/// under test runs).
fn wait_for_new_ok_prompt(m: &mut Machine, baseline: usize, max_fields: usize) -> String {
    const POLL_FIELDS: usize = 60;
    let mut fields = 0;
    loop {
        for _ in 0..POLL_FIELDS {
            m.run_field();
        }
        fields += POLL_FIELDS;
        if ok_prompt_count(m) > baseline {
            return m.text_screen_lines().join("\n");
        }
        assert!(
            fields < max_fields,
            "never saw a new OK prompt after {fields} fields; screen:\n{}",
            m.text_screen_lines().join("\n")
        );
    }
}

/// Boot to the `OK` prompt, type a 2-line program, `LLIST` it, and check the
/// captured bytes match exactly what BASIC's re-serialized listing sends over
/// the bit-banger port (CR line endings, un-translated).
#[test]
fn llist_captures_program_text_and_returns_to_ok_prompt() {
    /// Fields of BASIC settling before typing starts (matches `tests/fdc.rs`'s
    /// `boots_to_disk_basic_and_dir_lists_the_synthesized_file`).
    const BASIC_SETTLE_FIELDS: usize = 300;
    /// Generous upper bound on how long `LLIST` can take to finish and return
    /// to `OK`: at 600 baud (wiki `cocovm/bitbanger-spec` "Baud timing", 1486 cycles/
    /// bit x 10 bits/byte = 14860 cycles/byte) even a few hundred bytes of
    /// listing plus ROM tokenizing/detokenizing overhead is nowhere near this.
    const MAX_LLIST_FIELDS: usize = 6_000;

    let Some(rom) = load_rom() else {
        eprintln!("skipping bit-banger LLIST boot test: roms/coco3.rom not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();

    for _ in 0..BASIC_SETTLE_FIELDS {
        m.run_field();
    }

    // Attach the capture sink only once BASIC has settled at the `OK` prompt
    // — matching the real feature's usage (the menu's "Start Print Capture…"
    // always starts against an already-running machine) and sidestepping a
    // cold-boot artifact upstream of this sink:
    // the ROM's PIA1 DDRA setup ($A02F "LDX #$FF20" init routine) briefly
    // flips PA1 from mark to space for ~30 CPU cycles while reconfiguring the
    // pin direction, well under one bit-time (1486 cycles) and irrelevant to
    // any real print job, but enough to arm the decoder's start-bit edge
    // detector; with nothing else toggling the line for the next ~9.5
    // bit-times, that spurious edge free-runs to a bogus $FF byte. Attaching
    // the sink post-boot avoids depending on that unrelated cold-start
    // sequencing detail.
    let capture = CaptureSink::new();
    m.bus.bitbanger.set_sink(Box::new(capture.clone()));
    let baseline_ok = ok_prompt_count(&mut m);

    // A short 2-line program: LLIST only lists it back, never runs it, so the
    // GOTO is never actually taken.
    type_str(&mut m, "10 PRINT \"HELLO\"");
    tap_char(&mut m, '\r');
    type_str(&mut m, "20 GOTO 10");
    tap_char(&mut m, '\r');
    type_str(&mut m, "LLIST");
    tap_char(&mut m, '\r');

    // Wait for a new OK prompt: the machine must not hang during transmission,
    // such as by spinning forever on a wrongly polarized BUSY bit. It must
    // finish LLIST and return to the command loop.
    let screen = wait_for_new_ok_prompt(&mut m, baseline_ok, MAX_LLIST_FIELDS);
    assert!(
        !capture.bytes().is_empty(),
        "LLIST produced no bit-banger output at all; screen:\n{screen}"
    );

    let expected = b"10 PRINT \"HELLO\"\r20 GOTO 10\r";
    assert_eq!(
        capture.bytes(),
        expected,
        "captured LLIST bytes did not match the expected listing; got {:?}",
        String::from_utf8_lossy(&capture.bytes())
    );
}
