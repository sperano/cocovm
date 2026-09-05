//! Shared test harness: a TMS7040 on a [`FlatBoard`] with a ROM holding the
//! test program at `$F000`, already past its reset sequence.

use tms7000::{FlatBoard, ROM_BASE, ROM_SIZE, Step, StepKind, TMS7040, st};

pub const CODE: u16 = ROM_BASE;

pub struct Sys {
    pub cpu: TMS7040,
    pub board: FlatBoard,
}

// Each integration test compiles this harness independently, so not every helper
// is exercised by every suite — that's expected for shared test scaffolding.
#[allow(dead_code)]
impl Sys {
    /// A ROM with `code` at `$F000` and the reset vector pointing there.
    pub fn rom(code: &[u8]) -> Vec<u8> {
        let mut rom = vec![0; ROM_SIZE];
        rom[..code.len()].copy_from_slice(code);
        rom[ROM_SIZE - 2..].copy_from_slice(&CODE.to_be_bytes());
        rom
    }

    /// Boot a chip on `code`, leaving PC at `$F000`, SP at 1, A/B at 0.
    pub fn code(code: &[u8]) -> Self {
        Self::from_rom(&Self::rom(code))
    }

    /// Boot a chip on a full ROM image and run its reset sequence.
    pub fn from_rom(rom: &[u8]) -> Self {
        let mut s = Self {
            cpu: TMS7040::new(rom).expect("4 KB ROM"),
            board: FlatBoard::new(),
        };
        let reset = s.cpu.step(&mut s.board);
        assert_eq!(reset.kind, StepKind::Reset);
        s.board.writes.clear();
        s
    }

    pub fn step(&mut self) -> Step {
        self.cpu.step(&mut self.board)
    }

    /// Execute one instruction (skipping interrupt entries), returning its cycles.
    pub fn insn(&mut self) -> u32 {
        loop {
            let step = self.step();
            if step.kind == StepKind::Instruction {
                return step.cycles;
            }
        }
    }

    pub fn a(&self) -> u8 {
        self.cpu.a()
    }

    pub fn b(&self) -> u8 {
        self.cpu.b()
    }

    pub fn set_a(&mut self, v: u8) {
        self.cpu.set_rf(0, v);
    }

    pub fn set_b(&mut self, v: u8) {
        self.cpu.set_rf(1, v);
    }

    pub fn set_c(&mut self, on: bool) {
        if on {
            self.cpu.st |= st::C;
        } else {
            self.cpu.st &= !st::C;
        }
    }

    /// `(C, N, Z)`.
    pub fn flags(&self) -> (bool, bool, bool) {
        let s = self.cpu.st;
        (s & st::C != 0, s & st::N != 0, s & st::Z != 0)
    }
}
