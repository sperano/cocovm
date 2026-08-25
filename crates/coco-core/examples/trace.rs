//! Per-instruction CPU trace for trace-diffing against MAME.
//!
//! Two modes:
//! - No cart (`trace -- [max_instrs]`): the original deterministic cold-start
//!   trace, NO interrupts, comparable 1:1 against MAME up to the point BASIC
//!   first enables interrupts.
//! - With cart (`trace -- [max_instrs] <pak.ccc>`): inserts an autostart
//!   ROMPak and drives `Machine::step_instruction()` (which handles interrupt
//!   servicing before each instruction, hsync per line, vsync per field, and
//!   GIME timer ticks) so the full boot-and-run stream can be diffed against a
//!   MAME run with `-cart1 <pak.ccc>`.
//!
//! Usage: `cargo run -p coco-core --example trace -- [max_instrs] [cart.ccc]`

use std::io::{BufWriter, Write};

use coco_core::cart::ROMPak;
use coco_core::debug::TraceEntry;
use coco_core::{Machine, MachineConfig, StepKind};
use test_assets::rom::COCO3;

fn load_rom() -> Box<[u8]> {
    let path = test_assets::rom(COCO3);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn main() {
    let max: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let cart_path = std::env::args().nth(2);

    let mut m = Machine::new(MachineConfig::default(), load_rom());
    if let Some(path) = &cart_path {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        m.insert_cartridge(ROMPak::from_bytes(&bytes, true).unwrap());
        m.reset();
    }
    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());

    if cart_path.is_none() {
        // No-cart mode: deterministic cold-start trace, raw CPU steps with no
        // peripheral timing or interrupts.
        for _ in 0..max {
            let entry = TraceEntry::capture(&m.cpu);
            writeln!(w, "{}", entry.format()).unwrap();
            m.step_cpu_raw();
        }
        return;
    }

    // Cart mode: use step_instruction() which has full fidelity (interrupt
    // servicing, hsync, vsync, GIME timer). Trace before each instruction.
    let mut logged = 0usize;
    loop {
        let entry = TraceEntry::capture(&m.cpu);
        let event = m.step_instruction();
        if matches!(event.kind, StepKind::Instruction { .. }) {
            writeln!(w, "{}", entry.format()).unwrap();
            logged += 1;
            if logged >= max {
                break;
            }
        }
    }
}
