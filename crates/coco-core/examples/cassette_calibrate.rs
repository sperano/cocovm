//! Calibration probe: boot the real ROM, type a one-liner program, run
//! CSAVE"X", then dump the raw DAC transition capture
//! ([`coco_core::cassette::Cassette::capture`]) so the FSK timing (cycles/bit,
//! bit order, block framing) can be measured empirically — the CSAVE/CLOAD
//! bit-bang code lives in the undocumented $A000-$BFFF Color BASIC ROM, so it
//! can't be derived from local docs (`cassette-verified-facts` memory).
//!
//!     cargo run -p coco-core --example cassette_calibrate           # analysis
//!     cargo run -p coco-core --example cassette_calibrate -- --dump # raw CSV
//!
//! Findings from this probe (CSAVE"X" of `10 PRINT "HI"`, cross-checked by
//! decoding the captured waveform and matching it against the known tokenized
//! program bytes and block checksums — see `cassette.rs` demodulator):
//! - Each bit is one full DAC sine cycle: a 0-bit measures ~793 CPU cycles
//!   (~1128 Hz), a 1-bit ~455 cycles (~1967 Hz) — close to, but not exactly,
//!   the canonical 1200/2400 Hz (a hand-tuned ROM delay loop, not a crystal-
//!   locked tone; the ROM's own hysteresis demodulator tolerates the drift).
//! - Bit order is LSB-first.
//! - Block framing and checksum exactly match `cassette-verified-facts`:
//!   `$55* $3C type len data… checksum`, checksum = `sum(type,len,data) & 0xFF`.

use std::cmp::Reverse;

use coco_core::keyboard::char_key;
use coco_core::{Machine, MachineConfig};

const BOOT_FIELDS: u32 = 300;
const FIELDS_PER_KEY: u32 = 4;
const SAVE_FIELDS: u32 = 1200;

fn type_line(m: &mut Machine, text: &str) {
    for ch in text.chars().chain(std::iter::once('\r')) {
        let Some((pos, shifted)) = char_key(ch) else {
            panic!("no CoCo key for {ch:?}");
        };
        if shifted {
            m.bus.keyboard.set(coco_core::keyboard::SHIFT, true);
        }
        m.bus.keyboard.set(pos, true);
        for _ in 0..FIELDS_PER_KEY {
            m.run_field();
        }
        m.bus.keyboard.set(pos, false);
        m.bus.keyboard.set(coco_core::keyboard::SHIFT, false);
        for _ in 0..FIELDS_PER_KEY {
            m.run_field();
        }
    }
}

fn main() {
    let rom = std::fs::read("roms/coco3.rom").unwrap().into_boxed_slice();
    let mut m = Machine::new(MachineConfig::default(), rom);
    m.reset();
    for _ in 0..BOOT_FIELDS {
        m.run_field();
    }
    type_line(&mut m, "10 PRINT \"HI\"");
    type_line(&mut m, "CSAVE\"X\"");
    for _ in 0..SAVE_FIELDS {
        m.run_field();
        if m.bus.cassette.capture().len() > 200_000 {
            break; // safety valve
        }
    }

    let cap = m.bus.cassette.capture();
    if std::env::args().any(|a| a == "--dump") {
        for t in cap {
            println!("{},{}", t.cycle, t.level);
        }
        return;
    }
    println!("total transitions: {}", cap.len());
    if cap.is_empty() {
        println!("no transitions captured — motor never came on?");
        return;
    }
    println!("first cycle: {}, last cycle: {}", cap[0].cycle, cap[cap.len() - 1].cycle);
    println!("first 80 transitions (level, cycle, delta):");
    for i in 0..cap.len().min(80) {
        let delta = if i == 0 { 0 } else { cap[i].cycle - cap[i - 1].cycle };
        println!("{i:4}: level={:2} cycle={:8} delta={:5}", cap[i].level, cap[i].cycle, delta);
    }

    // Histogram of deltas (rounded) to spot the two tone periods.
    use std::collections::BTreeMap;
    let mut hist: BTreeMap<u64, u32> = BTreeMap::new();
    for i in 1..cap.len() {
        let delta = cap[i].cycle - cap[i - 1].cycle;
        *hist.entry(delta).or_insert(0) += 1;
    }
    println!("\ndelta histogram (delta_cycles: count), top 40 by count:");
    let mut entries: Vec<_> = hist.into_iter().collect();
    entries.sort_by_key(|&(_, count)| Reverse(count));
    for (delta, count) in entries.into_iter().take(40) {
        println!("{delta:6} : {count}");
    }

    // Distinct DAC levels used.
    use std::collections::BTreeSet;
    let levels: BTreeSet<u8> = cap.iter().map(|t| t.level).collect();
    println!("\ndistinct DAC levels used: {levels:?}");

    // Zero-crossing analysis: the SALT chip detects crossings of the AC-coupled
    // midpoint, not absolute 0. Use the observed level range's midpoint.
    let max_level = *levels.iter().max().unwrap();
    let mid = max_level / 2;
    println!("\nmax_level={max_level} mid={mid}");
    let mut crossings: Vec<(u64, bool)> = Vec::new(); // (cycle, rising)
    let mut side = cap[0].level > mid;
    for t in &cap[1..] {
        let new_side = t.level > mid;
        if new_side != side {
            crossings.push((t.cycle, new_side));
            side = new_side;
        }
    }
    println!("crossings: {}", crossings.len());
    println!("first 60 crossing deltas (cycle, rising, delta):");
    for i in 0..crossings.len().min(60) {
        let delta = if i == 0 { 0 } else { crossings[i].0 - crossings[i - 1].0 };
        println!("{i:4}: cycle={:8} rising={:5} delta={:5}", crossings[i].0, crossings[i].1, delta);
    }
    let mut chist: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
    for i in 1..crossings.len() {
        let delta = crossings[i].0 - crossings[i - 1].0;
        *chist.entry(delta).or_insert(0) += 1;
    }
    println!("\ncrossing-delta histogram, top 20:");
    let mut centries: Vec<_> = chist.into_iter().collect();
    centries.sort_by_key(|&(_, count)| Reverse(count));
    for (delta, count) in centries.into_iter().take(20) {
        println!("{delta:6} : {count}");
    }
}
