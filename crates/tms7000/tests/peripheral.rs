//! Peripheral file: port data/direction registers, IOCNT0's write
//! semantics, and the `P`-suffixed instructions that reach them.

mod common;

use common::Sys;
use tms7000::Port;

const IOCNT0: u8 = 0x00;
const APORT: u8 = 0x04;
const ADDR: u8 = 0x05;
const BPORT: u8 = 0x06;
const CPORT: u8 = 0x08;
const CDDR: u8 = 0x09;
const DPORT: u8 = 0x0A;
const DDDR: u8 = 0x0B;

/// `MOVP %imm,Pn`.
fn movp_imm(imm: u8, p: u8) -> [u8; 3] {
    [0xA2, imm, p]
}

/// `MOVP Pn,A`.
fn movp_to_a(p: u8) -> [u8; 2] {
    [0x80, p]
}

#[test]
fn port_write_is_masked_by_ddr_and_latched_unmasked() {
    let mut code = Vec::new();
    code.extend(movp_imm(0x0F, CDDR));
    code.extend(movp_imm(0xFF, CPORT));
    let mut s = Sys::code(&code);
    assert_eq!(s.insn(), 11, "MOVP %n,Pn");
    s.insn();
    assert_eq!(s.board.writes, vec![(Port::C, 0x0F)]);
    assert_eq!(s.cpu.port_latch(Port::C), 0xFF);
}

#[test]
fn port_read_mixes_input_pins_and_output_latch_by_ddr() {
    let mut code = Vec::new();
    code.extend(movp_imm(0xF0, DDDR));
    code.extend(movp_imm(0xAA, DPORT));
    code.extend(movp_to_a(DPORT));
    let mut s = Sys::code(&code);
    s.board.inputs[Port::D.index_for_test()] = 0x0F;
    s.insn();
    s.insn();
    assert_eq!(s.insn(), 9, "MOVP Pn,A");
    assert_eq!(s.a(), 0xAF, "latch on output bits, pins on input bits");
}

#[test]
fn port_b_is_output_only_and_reads_back_its_latch() {
    let mut code = Vec::new();
    code.extend(movp_imm(0x5A, BPORT));
    code.extend(movp_to_a(BPORT));
    let mut s = Sys::code(&code);
    s.board.inputs[1] = 0x00;
    s.insn();
    assert_eq!(s.board.writes, vec![(Port::B, 0x5A)]);
    s.insn();
    assert_eq!(s.a(), 0x5A);
}

#[test]
fn port_a_has_no_output_latch_or_ddr() {
    let mut code = Vec::new();
    code.extend(movp_imm(0xFF, ADDR));
    code.extend(movp_imm(0xFF, APORT));
    code.extend(movp_to_a(APORT));
    let mut s = Sys::code(&code);
    s.board.inputs[0] = 0x3C;
    s.insn();
    s.insn();
    assert!(s.board.writes.is_empty());
    s.insn();
    assert_eq!(s.a(), 0x3C, "reads the pins regardless");
    assert_eq!(s.cpu.port_ddr(Port::A), 0);
}

#[test]
fn ddr_change_does_not_refresh_the_pins() {
    let mut code = Vec::new();
    code.extend(movp_imm(0xFF, CPORT));
    code.extend(movp_imm(0xFF, CDDR));
    let mut s = Sys::code(&code);
    s.insn();
    s.insn();
    assert_eq!(
        s.board.writes,
        vec![(Port::C, 0x00)],
        "written while DDR was 0"
    );
}

#[test]
fn iocnt0_write_sets_enables_and_clears_flags_written_as_one() {
    let mut code = Vec::new();
    code.extend(movp_imm(0x15, IOCNT0)); // enable INT1/2/3
    code.extend(movp_imm(0x02, IOCNT0)); // clear INT1 flag, drop the enables
    code.extend(movp_to_a(IOCNT0));
    let mut s = Sys::code(&code);
    s.cpu.st = 0; // keep the global enable off so nothing dispatches
    s.insn();
    assert_eq!(s.cpu.io_control(), 0x15);
    s.cpu.set_int1(true);
    assert_eq!(s.cpu.io_control(), 0x17, "INT1 flag follows the line");
    s.cpu.set_int1(false);
    s.cpu.set_int1(true);
    s.insn();
    assert_eq!(
        s.cpu.io_control(),
        0x02,
        "line still high: flag springs back, enables gone"
    );
    s.insn();
    assert_eq!(s.a(), 0x02);
}

#[test]
fn andp_orp_xorp_and_btjop_reach_the_peripheral_file() {
    let mut code = Vec::new();
    code.extend(movp_imm(0xFF, CDDR));
    code.extend(movp_imm(0x00, CPORT)); // reset left the latch at $FF
    code.extend([0xA4, 0x81, CPORT]); // ORP %>81,P8
    code.extend([0xA3, 0x0F, CPORT]); // ANDP %>0F,P8
    code.extend([0xA5, 0xFF, CPORT]); // XORP %>FF,P8
    code.extend([0xA6, 0x01, CPORT, 0x02]); // BTJOP %>01,P8,+2
    code.extend([0x00, 0x00]);
    let mut s = Sys::code(&code);
    s.insn();
    s.insn();
    s.insn();
    assert_eq!(s.board.writes.last(), Some(&(Port::C, 0x81)));
    s.insn();
    assert_eq!(s.board.writes.last(), Some(&(Port::C, 0x01)));
    s.insn();
    assert_eq!(s.board.writes.last(), Some(&(Port::C, 0xFE)));
    assert_eq!(s.insn(), 11 + 2, "BTJOP not taken: bit 0 clear");
}

#[test]
fn unmapped_peripheral_registers_read_zero() {
    let mut s = Sys::code(&movp_to_a(0x07));
    s.set_a(0x99);
    s.insn();
    assert_eq!(s.a(), 0);

    let mut s = Sys::code(&movp_to_a(0x0C)); // past the 70x0's file
    s.set_a(0x99);
    s.insn();
    assert_eq!(s.a(), 0);
}

trait PortIndex {
    fn index_for_test(self) -> usize;
}

impl PortIndex for Port {
    fn index_for_test(self) -> usize {
        match self {
            Port::A => 0,
            Port::B => 1,
            Port::C => 2,
            Port::D => 3,
        }
    }
}
