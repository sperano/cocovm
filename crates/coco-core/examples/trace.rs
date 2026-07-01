//! Per-instruction CPU trace of the real ROM cold-start, for trace-diffing against
//! MAME (`coco3-super-extended-init-incomplete` bug hunt). Runs with NO interrupts
//! so the deterministic cold-start path can be compared 1:1 against MAME up to the
//! point BASIC first enables interrupts.
//!
//! Usage: `cargo run -p coco-core --example trace -- [max_instrs]`

use std::io::{BufWriter, Write};
use std::path::PathBuf;

use coco_core::{Machine, MachineConfig};

fn load_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn main() {
    let max: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let mut m = Machine::new(MachineConfig::default(), load_rom());
    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());

    for _ in 0..max {
        let c = &m.cpu;
        writeln!(
            w,
            "{:04X}:  A={:02X} B={:02X} X={:04X} Y={:04X} U={:04X} S={:04X} DP={:02X} CC={:02X}",
            c.pc, c.a, c.b, c.x, c.y, c.u, c.s, c.dp, c.cc
        )
        .unwrap();
        m.step();
    }
}
