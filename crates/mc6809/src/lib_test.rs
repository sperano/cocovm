use super::*;

#[test]
fn reset_loads_pc_from_vector_and_masks_interrupts() {
    let mut bus = FlatBus::new();
    bus.load(VECTOR_RESET, &[0x80, 0x00]); // reset vector -> $8000
    let mut cpu = MC6809::new();
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0x8000);
    assert_ne!(cpu.cc & cc::IRQ_MASK, 0);
    assert_ne!(cpu.cc & cc::FIRQ_MASK, 0);
}

#[test]
fn reset_clears_cycles_accumulated_by_prior_instructions() {
    const PROGRAM_START: u16 = 0x8000;
    const NOP_OPCODE: u8 = 0x12;
    const NOP_CYCLES: u64 = 2;
    const EXECUTED_INSTRUCTIONS: u64 = 2;

    let mut bus = FlatBus::new();
    bus.load(VECTOR_RESET, &PROGRAM_START.to_be_bytes());
    bus.load(PROGRAM_START, &[NOP_OPCODE, NOP_OPCODE]);
    let mut cpu = MC6809::new();

    cpu.reset(&mut bus);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.cycles, NOP_CYCLES * EXECUTED_INSTRUCTIONS);

    cpu.reset(&mut bus);
    assert_eq!(cpu.cycles, 0);
}

#[test]
fn lda_immediate_sets_a_and_zero_flag() {
    let mut bus = FlatBus::new();
    bus.load(0x0000, &[0x86, 0x00]); // LDA #$00
    let mut cpu = MC6809::new();
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert_eq!(cycles, 2);
    assert_ne!(cpu.cc & cc::ZERO, 0);
    assert_eq!(cpu.cc & cc::NEGATIVE, 0);
}

#[test]
fn d_pairs_a_and_b() {
    let mut cpu = MC6809::new();
    cpu.set_d(0x1234);
    assert_eq!(cpu.a, 0x12);
    assert_eq!(cpu.b, 0x34);
    assert_eq!(cpu.d(), 0x1234);
}
