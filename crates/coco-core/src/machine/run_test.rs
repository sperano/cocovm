use super::*;
use crate::MachineConfig;
use crate::gime::{init0, intr};
use mc6809::{VECTOR_IRQ, cc};

const ROM_BASE: u16 = 0x8000;
const ROM_SIZE: usize = 32 * 1024;
const HANDLER_ADDR: u16 = ROM_BASE;
const STACK_TOP: u16 = 0x2000;
const NOP_OPCODE: u8 = 0x12;
const NOP_CYCLES: u32 = 2;
const IRQ_ENTRY_CYCLES: u32 = 19;
const VECTOR_BYTES: usize = 2;

fn machine_with_pending_irq() -> Machine {
    let mut rom = vec![0; ROM_SIZE];
    rom[usize::from(HANDLER_ADDR - ROM_BASE)] = NOP_OPCODE;
    let vector_offset = usize::from(VECTOR_IRQ - ROM_BASE);
    rom[vector_offset..vector_offset + VECTOR_BYTES].copy_from_slice(&HANDLER_ADDR.to_be_bytes());

    let mut machine = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
    machine.cpu.s = STACK_TOP;
    machine.cpu.cc &= !cc::IRQ_MASK;
    machine.bus.gime.write_init0(init0::IEN);
    machine.bus.gime.write_irq_enable(intr::TMR);
    machine.bus.gime.raise(intr::TMR);
    machine
}

#[test]
fn interrupt_entry_advances_machine_timing() {
    let mut machine = machine_with_pending_irq();
    let cpu_cycles_before = machine.cpu.cycles;
    let bus_cycles_before = machine.bus.cycle_clock;
    let expected_cycles = IRQ_ENTRY_CYCLES + NOP_CYCLES;

    let event = machine.step_instruction();

    assert_eq!(
        event.kind,
        StepKind::Instruction {
            cycles: expected_cycles
        }
    );
    assert_eq!(
        machine.cpu.cycles - cpu_cycles_before,
        u64::from(expected_cycles)
    );
    assert_eq!(
        machine.bus.cycle_clock - bus_cycles_before,
        u64::from(expected_cycles)
    );
    assert_eq!(machine.line_cycles_spent, expected_cycles);
}
