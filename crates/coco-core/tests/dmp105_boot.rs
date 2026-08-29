//! End-to-end smoke test for the DMP-105 interpreter (T4): boot the real Super
//! Extended Color BASIC ROM, `LLIST` a one-liner
//! through the bit-banger port with a [`DMP105Handle`] attached as the sink,
//! and assert the paper picked up plausible content — proving the whole
//! chain (PIA1 bit-bang TX -> `BitBanger` decode -> `DMP105` interpretation
//! -> `Paper`) works against unmodified ROM code without panicking. Glyph-
//! exact assertions are the unit golden tests' job (`src/dmp105.rs`); this
//! only checks shape: nonzero dots, a plausible line count, no hangs.
//!
//! Skips gracefully if `roms/` isn't present, matching
//! `tests/bitbanger_boot.rs`.

use coco_core::bitbanger::PrinterSink;
use coco_core::dmp105::DMP105Handle;
use coco_core::printer::Y_UNITS_PER_INCH;
use coco_core::{Machine, MachineConfig};
use test_assets::rom::COCO3;

fn load_rom() -> Option<Box<[u8]>> {
    let path = test_assets::rom(COCO3);
    std::fs::read(&path).ok().map(Vec::into_boxed_slice)
}

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

fn ok_prompt_count(m: &mut Machine) -> usize {
    m.text_screen_lines()
        .iter()
        .filter(|line| line.trim_end() == "OK")
        .count()
}

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

#[test]
fn llist_through_dmp105_produces_plausible_paper_content() {
    const BASIC_SETTLE_FIELDS: usize = 300;
    const MAX_LLIST_FIELDS: usize = 6_000;

    let Some(rom) = load_rom() else {
        eprintln!("skipping DMP-105 LLIST boot test: roms/coco3.rom not present");
        return;
    };

    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();

    for _ in 0..BASIC_SETTLE_FIELDS {
        m.run_field();
    }

    // Attach the DMP-105 sink only once BASIC has settled, sidestepping the
    // same cold-boot PIA1 DDRA reconfiguration artifact `bitbanger_boot.rs`
    // documents (a spurious sub-bit-time edge that could otherwise arm the
    // decoder before any real transmission).
    let dmp = m.bus.bitbanger.start_dmp105();
    let baseline_ok = ok_prompt_count(&mut m);

    type_str(&mut m, "10 PRINT \"HELLO\"");
    tap_char(&mut m, '\r');
    type_str(&mut m, "LLIST");
    tap_char(&mut m, '\r');

    let screen = wait_for_new_ok_prompt(&mut m, baseline_ok, MAX_LLIST_FIELDS);

    let extent = dmp.paper_extent();
    assert!(
        extent.dot_count > 0,
        "LLIST produced no dots on the DMP-105's paper at all; screen:\n{screen}"
    );

    // `10 PRINT "HELLO"` is 16 mostly nonblank characters; a plausible dot
    // count for one line at this font's density is comfortably in the
    // hundreds. `max_y` only reflects rows an actual glyph dot landed on
    // (control codes such as the trailing CR move the head without marking
    // anything), so it stays within the single 9x7 glyph cell's body
    // rows here — a single-line listing never gets far enough to trigger a
    // second line feed.
    assert!(
        extent.dot_count > 50,
        "expected a plausible amount of ink for one line of listing text, got {}",
        extent.dot_count
    );
    assert!(
        extent.max_y < Y_UNITS_PER_INCH / 6,
        "a single printed line's dots should stay within one glyph cell's body rows, got max_y={}",
        extent.max_y
    );

    // A fresh DMP105Handle used directly as a PrinterSink must also accept
    // bytes without panicking. This covers the trait-object path the real bus
    // uses.
    let direct = DMP105Handle::new();
    let mut sink: Box<dyn PrinterSink> = Box::new(direct.clone());
    for &b in b"SANITY\r" {
        sink.write_byte(b);
    }
    assert!(direct.paper_extent().dot_count > 0);
}
