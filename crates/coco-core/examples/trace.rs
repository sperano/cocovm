//! Per-instruction CPU trace for trace-diffing against MAME.
//!
//! Two modes:
//! - No cart (`trace -- [max_instrs]`): the original deterministic cold-start
//!   trace, NO interrupts, comparable 1:1 against MAME up to the point BASIC
//!   first enables interrupts.
//! - With cart (`trace -- [max_instrs] <pak.ccc>`): inserts an autostart
//!   RomPak and replicates `Machine::run_field`'s scanline loop (interrupt
//!   servicing before each instruction, hsync per line, vsync per field, GIME
//!   timer ticks) so the full boot-and-run stream can be diffed against a MAME
//!   run with `-cart1 <pak.ccc>`.
//!
//! Usage: `cargo run -p coco-core --example trace -- [max_instrs] [cart.ccc]`

use std::io::{BufWriter, Write};
use std::path::PathBuf;

use coco_core::cart::RomPak;
use coco_core::{Machine, MachineConfig};

/// NTSC field structure mirrored from `Machine::run_field` (private consts).
const LINES_PER_FIELD: u32 = 262;
/// ~0.895 MHz / 60 Hz / 262 lines; doubled when the GIME fast-CPU poke is on.
const CYCLES_PER_LINE_NORMAL: u32 = 57;
/// TINS=1 counts the 3.58 MHz clock: 4 ticks per normal-speed CPU cycle.
const FAST_TIMER_TICKS_PER_LINE: u32 = CYCLES_PER_LINE_NORMAL * 4;

fn load_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn log_state(w: &mut impl Write, m: &Machine) {
    let c = &m.cpu;
    writeln!(
        w,
        "{:04X}:  A={:02X} B={:02X} X={:04X} Y={:04X} U={:04X} S={:04X} DP={:02X} CC={:02X}",
        c.pc, c.a, c.b, c.x, c.y, c.u, c.s, c.dp, c.cc
    )
    .unwrap();
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
        m.insert_cartridge(Box::new(RomPak::from_bytes(&bytes, true).unwrap()));
        m.reset();
    }
    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());

    if cart_path.is_none() {
        for _ in 0..max {
            log_state(&mut w, &m);
            m.step();
        }
        return;
    }

    // Cart mode: faithful replica of Machine::run_field / run_cycles /
    // service_interrupts, with a trace line before every instruction.
    let fs_falling_line = m.config.video.fs_falling_line(m.config.variant);
    let fs_rising_line = m.config.video.fs_rising_line(m.config.variant);
    let mut logged = 0usize;
    'trace: loop {
        for line in 0..LINES_PER_FIELD {
            let cycles_per_line = if m.bus.gime.cpu_fast {
                CYCLES_PER_LINE_NORMAL * 2
            } else {
                CYCLES_PER_LINE_NORMAL
            };
            let mut spent = 0u32;
            while spent < cycles_per_line {
                if m.bus.firq_asserted() {
                    m.cpu.firq(&mut m.bus);
                }
                if m.bus.irq_asserted() {
                    m.cpu.irq(&mut m.bus);
                }
                log_state(&mut w, &m);
                logged += 1;
                if logged >= max {
                    break 'trace;
                }
                spent += m.step();
            }
            m.bus.hsync();
            if line == fs_falling_line {
                m.bus.fs_falling();
            }
            if line == fs_rising_line {
                m.bus.fs_rising();
            }
            // 4 ticks/cycle at normal speed, 2 at double: per-line total is
            // the same 228 either way (matches run_field's arithmetic).
            let ticks = if m.bus.gime.timer_is_fast() {
                FAST_TIMER_TICKS_PER_LINE
            } else {
                1
            };
            m.bus.gime.tick_timer(ticks);
        }
    }
}
