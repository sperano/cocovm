//! Hardware-compatible aliases in the MC6809 read-modify-write opcode group.

mod common;

use common::Sys;
use mc6809::{Bus, MC6809, cc};

const PROGRAM_ADDRESS: u16 = 0x1000;
const DIRECT_ADDRESS: u16 = 0x0040;
const INDEXED_ADDRESS: u16 = 0x1234;

#[derive(Debug, PartialEq, Eq)]
enum Access {
    Read(u16),
    Write(u16, u8),
}

struct TraceBus {
    mem: Box<[u8; 0x10000]>,
    accesses: Vec<Access>,
}

impl TraceBus {
    fn new(program: &[u8], target: u16, value: u8) -> Self {
        let mut bus = Self {
            mem: Box::new([0; 0x10000]),
            accesses: Vec::new(),
        };
        bus.mem[PROGRAM_ADDRESS as usize..][..program.len()].copy_from_slice(program);
        bus.mem[target as usize] = value;
        bus
    }

    fn take_accesses(&mut self) -> Vec<Access> {
        std::mem::take(&mut self.accesses)
    }
}

impl Bus for TraceBus {
    fn read(&mut self, address: u16) -> u8 {
        self.accesses.push(Access::Read(address));
        self.mem[address as usize]
    }

    fn write(&mut self, address: u16, value: u8) {
        self.accesses.push(Access::Write(address, value));
        self.mem[address as usize] = value;
    }
}

#[test]
fn direct_neg_alias_reads_writes_and_updates_value() {
    let mut s = Sys::code(0x1000, &[0x01, 0x40]);
    s.set_mem(0x0040, 0x01);

    assert_eq!(s.step(), 6);
    assert_eq!(s.mem(0x0040), 0xFF);
    assert_ne!(s.cpu.cc & cc::NEGATIVE, 0);
    assert_ne!(s.cpu.cc & cc::CARRY, 0);
}
#[test]
fn direct_xnc_selects_neg_or_com_from_carry() {
    let mut neg = Sys::code(0x1000, &[0x02, 0x40]);
    neg.set_mem(0x0040, 0x01);
    neg.cpu.cc &= !cc::CARRY;
    neg.step();
    assert_eq!(neg.mem(0x0040), 0xFF);

    let mut com = Sys::code(0x1000, &[0x02, 0x40]);
    com.set_mem(0x0040, 0x01);
    com.cpu.cc |= cc::CARRY;
    com.step();
    assert_eq!(com.mem(0x0040), 0xFE);
    assert_ne!(com.cpu.cc & cc::CARRY, 0);
}

#[test]
fn direct_xdec_sets_carry_from_original_value() {
    let mut nonzero = Sys::code(0x1000, &[0x0B, 0x40]);
    nonzero.set_mem(0x0040, 0x01);
    nonzero.step();
    assert_eq!(nonzero.mem(0x0040), 0x00);
    assert_ne!(nonzero.cpu.cc & cc::CARRY, 0);

    let mut zero = Sys::code(0x1000, &[0x0B, 0x40]);
    zero.set_mem(0x0040, 0x00);
    zero.step();
    assert_eq!(zero.mem(0x0040), 0xFF);
    assert_eq!(zero.cpu.cc & cc::CARRY, 0);
}

#[test]
fn accumulator_xclr_preserves_carry_and_clears_accumulator() {
    let mut s = Sys::code(0x1000, &[0x4E]);
    s.cpu.a = 0x7F;
    s.cpu.cc |= cc::CARRY;

    assert_eq!(s.step(), 2);
    assert_eq!(s.cpu.a, 0);
    assert_ne!(s.cpu.cc & cc::ZERO, 0);
    assert_eq!(s.cpu.cc & cc::CARRY, cc::CARRY);
}

#[test]
fn direct_aliases_have_ordered_read_modify_write_bus_accesses() {
    for (opcode, input, output) in [
        (0x01, 0x01, 0xFF),
        (0x02, 0x01, 0xFF),
        (0x05, 0x03, 0x01),
        (0x0B, 0x02, 0x01),
    ] {
        let mut bus = TraceBus::new(&[opcode, DIRECT_ADDRESS as u8], DIRECT_ADDRESS, input);
        let mut cpu = MC6809::new();
        cpu.pc = PROGRAM_ADDRESS;
        assert_eq!(cpu.step(&mut bus), 6, "opcode {opcode:#04X}");
        assert_eq!(
            bus.take_accesses(),
            vec![
                Access::Read(PROGRAM_ADDRESS),
                Access::Read(PROGRAM_ADDRESS + 1),
                Access::Read(DIRECT_ADDRESS),
                Access::Write(DIRECT_ADDRESS, output),
            ],
            "opcode {opcode:#04X}"
        );
    }
}

#[test]
fn xnc_carry_selects_com_and_preserves_order() {
    let mut bus = TraceBus::new(&[0x02, DIRECT_ADDRESS as u8], DIRECT_ADDRESS, 0x01);
    let mut cpu = MC6809::new();
    cpu.pc = PROGRAM_ADDRESS;
    cpu.cc |= cc::CARRY;

    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(
        bus.take_accesses(),
        vec![
            Access::Read(PROGRAM_ADDRESS),
            Access::Read(PROGRAM_ADDRESS + 1),
            Access::Read(DIRECT_ADDRESS),
            Access::Write(DIRECT_ADDRESS, 0xFE),
        ]
    );
}

#[test]
fn indexed_alias_includes_postbyte_before_target_accesses() {
    let mut bus = TraceBus::new(&[0x61, 0x84], INDEXED_ADDRESS, 0x01);
    let mut cpu = MC6809::new();
    cpu.pc = PROGRAM_ADDRESS;
    cpu.x = INDEXED_ADDRESS;

    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(
        bus.take_accesses(),
        vec![
            Access::Read(PROGRAM_ADDRESS),
            Access::Read(PROGRAM_ADDRESS + 1),
            Access::Read(INDEXED_ADDRESS),
            Access::Write(INDEXED_ADDRESS, 0xFF),
        ]
    );
}

#[test]
fn direct_tst_reads_target_without_write() {
    let mut bus = TraceBus::new(&[0x0D, DIRECT_ADDRESS as u8], DIRECT_ADDRESS, 0x01);
    let mut cpu = MC6809::new();
    cpu.pc = PROGRAM_ADDRESS;

    assert_eq!(cpu.step(&mut bus), 6);
    assert_eq!(
        bus.take_accesses(),
        vec![
            Access::Read(PROGRAM_ADDRESS),
            Access::Read(PROGRAM_ADDRESS + 1),
            Access::Read(DIRECT_ADDRESS),
        ]
    );
}
