//! SAM R1 CPU-rate strobe ($FFD8/$FFD9 — `POKE 65497,0`): true double speed
//! on the CoCo 3. R0 ($FFD6/$FFD7) is inert (SEB Unravelled II Fig 8).

use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const R0_SET: u16 = 0xFFD7;
const R1_CLEAR: u16 = 0xFFD8;
const R1_SET: u16 = 0xFFD9;

#[test]
fn r1_strobes_latch_cpu_speed_and_r0_is_inert() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    assert!(!b.gime.cpu_fast);
    b.write(R1_SET, 0);
    assert!(b.gime.cpu_fast);
    b.write(R1_CLEAR, 0);
    assert!(!b.gime.cpu_fast);
    b.write(R0_SET, 0); // CoCo 1/2 address-dependent speed: does nothing here
    assert!(!b.gime.cpu_fast);
}

#[test]
fn speed_poke_doubles_cycles_per_field() {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    // Park the CPU on BRA * so fields execute nothing but the idle loop.
    m.bus.write(0x0000, 0x20);
    m.bus.write(0x0001, 0xFE);

    m.run_field();
    let start = m.cpu.cycles;
    m.run_field();
    let slow = m.cpu.cycles - start;

    m.bus.write(R1_SET, 0);
    let start = m.cpu.cycles;
    m.run_field();
    let fast = m.cpu.cycles - start;

    let ratio = fast as f64 / slow as f64;
    assert!(
        (1.9..=2.1).contains(&ratio),
        "double speed should run ~2x cycles per field: slow={slow} fast={fast}"
    );
}
