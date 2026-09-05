//! Instruction trace of the Sound/Speech Cartridge's TMS7040 on the real
//! board (RAM, AY, SP0256), for diffing against MAME's `pic7040` trace
//! with `scripts/ssc-trace-diff.py`. Same line format as
//! `crates/tms7000/examples/firmware_trace.rs`.
//!
//! Usage: `cargo run -p coco-core --example ssc_trace -- [--max N]
//! [--at CYCLE:BYTE]... [--cycles] [--dump-ram]`
//!
//! `--at` writes `BYTE` to `$FF7E` once the firmware's cycle counter passes
//! `CYCLE`, the way the CoCo would poke it.

use std::io::{BufWriter, Write};

use coco_core::cart::Cartridge;
use coco_core::ssc::{SoundSpeechCartridge, reg};
use test_assets::rom::{SP0256_AL2, SSC_TMS7040};
use tms7000::disasm;

/// E-cycles per cartridge tick, about one 6809 instruction.
const TICK_CYCLES: u32 = 8;
/// Firmware instructions kept between drains.
const TRACE_CAPACITY: usize = 1024;

struct Args {
    max: u64,
    host_bytes: Vec<(u64, u8)>,
    cycles: bool,
    dump_ram: bool,
}

fn parse_args() -> Args {
    let mut args = Args {
        max: 200_000,
        host_bytes: Vec::new(),
        cycles: false,
        dump_ram: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--cycles" {
            args.cycles = true;
            continue;
        }
        if flag == "--dump-ram" {
            args.dump_ram = true;
            continue;
        }
        let value = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--max" => args.max = value.parse().expect("--max N"),
            "--at" => {
                let (cycle, byte) = value.split_once(':').expect("--at CYCLE:BYTE");
                args.host_bytes.push((
                    cycle.parse().expect("cycle"),
                    u8::from_str_radix(byte.trim_start_matches("0x"), 16).expect("hex byte"),
                ));
            }
            other => panic!("unknown flag {other}"),
        }
    }
    args.host_bytes.sort_unstable();
    args
}

fn read_rom(name: &str) -> Vec<u8> {
    let path = test_assets::rom(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn main() {
    let args = parse_args();
    let mut ssc = SoundSpeechCartridge::new(&read_rom(SSC_TMS7040), &read_rom(SP0256_AL2))
        .expect("installed SSC ROMs");
    ssc.enable_firmware_trace(TRACE_CAPACITY);

    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());
    let mut pending = args.host_bytes.into_iter().peekable();
    let mut lines = 0u64;
    while lines < args.max {
        if let Some(&(cycle, byte)) = pending.peek()
            && ssc.firmware().cycles >= cycle
        {
            ssc.write(reg::DATA, byte);
            pending.next();
        }
        ssc.tick(TICK_CYCLES);
        for entry in ssc.drain_firmware_trace() {
            let insn = disasm::disassemble(&mut |addr| ssc.firmware().peek(addr), entry.pc);
            write!(
                w,
                "A={:02X} B={:02X} ST={:02X} SP={:02X} {:04X}: {insn}",
                entry.a, entry.b, entry.st, entry.sp, entry.pc
            )
            .unwrap();
            if args.cycles {
                write!(w, " ;cycles={}", entry.cycles).unwrap();
            }
            writeln!(w).unwrap();
            lines += 1;
            if lines >= args.max {
                break;
            }
        }
    }
    if ssc.firmware().illegal_count > 0 {
        eprintln!("illegal opcodes executed: {}", ssc.firmware().illegal_count);
    }
    if args.dump_ram {
        for (addr, byte) in ssc.ram().iter().enumerate().filter(|(_, b)| **b != 0) {
            eprintln!("RAM {addr:03X}: {byte:02X}");
        }
    }
}
