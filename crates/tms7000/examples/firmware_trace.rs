//! Instruction trace of the Sound/Speech Cartridge firmware on a stub
//! board, for diffing against MAME's `pic7040` debugger trace.
//!
//! Each line: `A=xx B=xx ST=xx SP=xx PPPP: DISASM`, the format MAME prints
//! for `trace <file>,<cpu>,,{tracelog "A=%02X B=%02X ST=%02X SP=%02X ",a,b,st,sp}`.
//!
//! Usage: `cargo run -p tms7000 --example firmware_trace -- [--max N]
//! [--at CYCLE:BYTE]... [--reset-at CYCLE]`
//!
//! `--at` latches `BYTE` into port A and raises INT3 once the chip's cycle
//! counter passes `CYCLE` (the firmware's port A read drops it again);
//! `--reset-at` pulses the RESET pin. INT1 (the speech chip's load request)
//! is held high, as on an idle SP0256.

use std::io::{BufWriter, Write};

use tms7000::{Bus, Port, StepKind, TMS7040, disasm};

/// Stub of the cartridge: a port A latch whose read drops INT3.
struct StubBoard {
    port_a: u8,
    int3: bool,
}

impl Bus for StubBoard {
    fn read_port(&mut self, port: Port) -> u8 {
        match port {
            Port::A => {
                self.int3 = false;
                self.port_a
            }
            _ => 0xFF,
        }
    }

    fn write_port(&mut self, _port: Port, _val: u8) {}
}

struct Args {
    max: u64,
    host_bytes: Vec<(u64, u8)>,
    reset_at: Option<u64>,
}

fn parse_args() -> Args {
    let mut args = Args {
        max: 200_000,
        host_bytes: Vec::new(),
        reset_at: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
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
            "--reset-at" => args.reset_at = Some(value.parse().expect("--reset-at CYCLE")),
            other => panic!("unknown flag {other}"),
        }
    }
    args.host_bytes.sort_unstable();
    args
}

fn main() {
    let args = parse_args();
    let path = test_assets::rom(test_assets::rom::SSC_TMS7040);
    let rom =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut cpu = TMS7040::new(&rom).expect("4 KB firmware");
    let mut board = StubBoard {
        port_a: 0,
        int3: false,
    };
    cpu.set_int1(true);

    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());
    let mut pending = args.host_bytes.into_iter().peekable();
    let mut reset_at = args.reset_at;
    let mut lines = 0u64;
    while lines < args.max {
        if let Some(&(cycle, byte)) = pending.peek()
            && cpu.cycles >= cycle
        {
            board.port_a = byte;
            board.int3 = true;
            cpu.set_int3(true);
            pending.next();
        }
        if reset_at.is_some_and(|cycle| cpu.cycles >= cycle) {
            cpu.reset(&mut board);
            reset_at = None;
        }
        let pc = cpu.pc;
        let (a, b, st, sp) = (cpu.a(), cpu.b(), cpu.st, cpu.sp);
        let step = cpu.step(&mut board);
        cpu.set_int3(board.int3);
        if step.kind != StepKind::Instruction {
            continue;
        }
        let insn = disasm::disassemble(&mut |addr| cpu.peek(addr), pc);
        writeln!(
            w,
            "A={a:02X} B={b:02X} ST={st:02X} SP={sp:02X} {pc:04X}: {insn}"
        )
        .unwrap();
        lines += 1;
    }
    if cpu.illegal_count > 0 {
        eprintln!("illegal opcodes executed: {}", cpu.illegal_count);
    }
}
