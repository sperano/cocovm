use super::*;

const EXPECTED_FULL_INTERRUPT_CYCLES: u64 = 19;
const EXPECTED_FAST_INTERRUPT_CYCLES: u64 = 10;
const EXPECTED_CWAI_WAKE_CYCLES: u64 = 4;
const INITIAL_CYCLES: u64 = 37;
const STACK_TOP: u16 = 0x2000;
const CWAI_STACK_POINTER: u16 = 0x1FF4;
const IRQ_HANDLER: u16 = 0x8000;
const FIRQ_HANDLER: u16 = 0x8100;
const NMI_HANDLER: u16 = 0x8200;
const SWI_PROGRAM_START: u16 = 0x1000;
const SWI_OPCODE: u8 = 0x3F;

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

#[test]
fn external_interrupts_charge_their_entry_costs() {
    let mut bus = FlatBus::new();
    bus.load(VECTOR_IRQ, &IRQ_HANDLER.to_be_bytes());
    bus.load(VECTOR_FIRQ, &FIRQ_HANDLER.to_be_bytes());
    bus.load(VECTOR_NMI, &NMI_HANDLER.to_be_bytes());

    let mut irq_cpu = MC6809::new();
    irq_cpu.s = STACK_TOP;
    irq_cpu.cycles = INITIAL_CYCLES;
    assert!(irq_cpu.irq(&mut bus));
    assert_eq!(
        irq_cpu.cycles,
        INITIAL_CYCLES + EXPECTED_FULL_INTERRUPT_CYCLES
    );

    let mut firq_cpu = MC6809::new();
    firq_cpu.s = STACK_TOP;
    firq_cpu.cycles = INITIAL_CYCLES;
    assert!(firq_cpu.firq(&mut bus));
    assert_eq!(
        firq_cpu.cycles,
        INITIAL_CYCLES + EXPECTED_FAST_INTERRUPT_CYCLES
    );

    let mut nmi_cpu = MC6809::new();
    nmi_cpu.s = STACK_TOP;
    nmi_cpu.nmi_armed = true;
    nmi_cpu.cycles = INITIAL_CYCLES;
    nmi_cpu.nmi(&mut bus);
    assert_eq!(
        nmi_cpu.cycles,
        INITIAL_CYCLES + EXPECTED_FULL_INTERRUPT_CYCLES
    );
}

#[test]
fn ignored_external_interrupts_do_not_charge_cycles() {
    let mut bus = FlatBus::new();
    let mut cpu = MC6809::new();
    cpu.cc = cc::IRQ_MASK | cc::FIRQ_MASK;
    cpu.cycles = INITIAL_CYCLES;

    assert!(!cpu.irq(&mut bus));
    assert!(!cpu.firq(&mut bus));
    cpu.nmi(&mut bus);
    assert_eq!(cpu.cycles, INITIAL_CYCLES);
}

#[test]
fn cwai_wake_charges_only_the_vector_sequence() {
    let mut bus = FlatBus::new();
    bus.load(VECTOR_IRQ, &IRQ_HANDLER.to_be_bytes());
    let mut cpu = MC6809::new();
    cpu.state = State::Waiting;
    cpu.s = CWAI_STACK_POINTER;
    cpu.cycles = INITIAL_CYCLES;

    assert!(cpu.irq(&mut bus));
    assert_eq!(
        cpu.s, CWAI_STACK_POINTER,
        "CWAI wake must not stack another frame"
    );
    assert_eq!(cpu.cycles, INITIAL_CYCLES + EXPECTED_CWAI_WAKE_CYCLES);
}

#[test]
fn software_interrupt_is_not_double_charged() {
    const SWI_CYCLES: u64 = 19;
    let mut bus = FlatBus::new();
    bus.load(SWI_PROGRAM_START, &[SWI_OPCODE]);
    bus.load(VECTOR_SWI, &IRQ_HANDLER.to_be_bytes());
    let mut cpu = MC6809::new();
    cpu.pc = SWI_PROGRAM_START;
    cpu.s = STACK_TOP;

    assert_eq!(u64::from(cpu.step(&mut bus)), SWI_CYCLES);
    assert_eq!(cpu.cycles, SWI_CYCLES);
}
