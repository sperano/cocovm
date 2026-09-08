//! Synthetic, redistributable inputs shared by core and native benchmarks.

use coco_core::Machine;
use coco_core::orch90::{LEFT_DAC_REG, Orch90, RIGHT_DAC_REG};
use mc6809::{Bus, cc};

pub const ROM_BYTES: usize = 32 * 1024;
const CART_ROM_BYTES: usize = 8 * 1024;
pub const PROGRAM_START: u16 = 0x1000;
const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_DATA: u16 = 0xFF20;
const PIA1_CRA: u16 = 0xFF21;
const PIA1_CRB: u16 = 0xFF23;
const DDR_ACCESS: u8 = 0x30;
const DATA_C2_LOW: u8 = 0x34;
const DATA_C2_HIGH: u8 = 0x3C;
const DAC_OUTPUT_BITS: u8 = 0xFC;
pub const DAC_INCREMENT: u8 = 4;
const STA_EXTENDED: u8 = 0xB7;
const ADDA_IMMEDIATE: u8 = 0x8B;
const BRA: u8 = 0x20;
const DAC_LOOP_BACK: u8 = (-7_i8) as u8;
const CART_LOOP_BACK: u8 = (-10_i8) as u8;

fn install(machine: &mut Machine, program: &[u8]) {
    for (offset, &byte) in program.iter().enumerate() {
        machine.bus.write(PROGRAM_START + offset as u16, byte);
    }
    machine.cpu.pc = PROGRAM_START;
    machine.cpu.a = 0;
    machine.cpu.cc |= cc::IRQ_MASK | cc::FIRQ_MASK;
}

/// Service Manual pp. 9–10, 43: PA2–7 output, SEL=00, SNDEN high.
pub fn configure_dac(machine: &mut Machine) {
    for (address, value) in [
        (PIA1_CRA, DDR_ACCESS),
        (PIA1_DATA, DAC_OUTPUT_BITS),
        (PIA1_CRA, DATA_C2_LOW),
        (PIA0_CRA, DATA_C2_LOW),
        (PIA0_CRB, DATA_C2_LOW),
        (PIA1_CRB, DATA_C2_HIGH),
    ] {
        machine.bus.write(address, value);
    }
    let [high, low] = PIA1_DATA.to_be_bytes();
    install(
        machine,
        &[
            STA_EXTENDED,
            high,
            low,
            ADDA_IMMEDIATE,
            DAC_INCREMENT,
            BRA,
            DAC_LOOP_BACK,
        ],
    );
}

/// Uses the same synthetic 8 KiB cartridge as `tests/orch90.rs`.
pub fn configure_cartridge(machine: &mut Machine) {
    machine.bus.cart = Orch90::from_rom_bytes(&[0; CART_ROM_BYTES]).unwrap().into();
    let [lh, ll] = LEFT_DAC_REG.to_be_bytes();
    let [rh, rl] = RIGHT_DAC_REG.to_be_bytes();
    install(
        machine,
        &[
            STA_EXTENDED,
            lh,
            ll,
            ADDA_IMMEDIATE,
            DAC_INCREMENT,
            STA_EXTENDED,
            rh,
            rl,
            BRA,
            CART_LOOP_BACK,
        ],
    );
}

pub fn assert_changing_audio(machine: &mut Machine) {
    machine.take_audio().count();
    machine.run_field();
    let samples: Vec<_> = machine.take_audio().collect();
    assert!(!samples.is_empty(), "workload produced no audio frames");
    assert!(samples.iter().flatten().all(|v| v.is_finite()));
    assert!(samples.iter().flatten().any(|v| *v > 0.0));
    assert!(
        samples.windows(2).any(|pair| pair[0] != pair[1]),
        "workload audio is constant"
    );
}

/// HSCREEN 2 geometry with guest writes into the MMU-off visible RAM window.
pub fn configure_graphics(machine: &mut Machine) {
    const INIT0: u16 = 0xFF90;
    const NATIVE_MMU_OFF: u8 = 0x02;
    const VIDEO_MODE: u16 = 0xFF98;
    const GRAPHICS_REGISTERS: [u8; 8] = [0x80, 0x1E, 0, 0, 0, 0xE4, 0, 0];
    const PALETTE_BASE: u16 = 0xFFB0;
    const VIDEO_BASE: usize = 0x72000;
    const VIDEO_BYTES: usize = 160 * 192;
    const PALETTE_STEP: u8 = 4;
    machine.bus.write(INIT0, NATIVE_MMU_OFF);
    for (offset, value) in GRAPHICS_REGISTERS.into_iter().enumerate() {
        machine.bus.write(VIDEO_MODE + offset as u16, value);
    }
    for color in 0..16_u8 {
        machine
            .bus
            .write(PALETTE_BASE + u16::from(color), color * PALETTE_STEP);
    }
    for (index, byte) in machine.bus.ram[VIDEO_BASE..VIDEO_BASE + VIDEO_BYTES]
        .iter_mut()
        .enumerate()
    {
        *byte = index as u8;
    }
    // Animate the chosen $6000-byte span, covering the first 153 full rows.
    // LDX #$2000; STA ,X+; INCA; CMPX #$8000; BNE store; INCA; BRA start.
    // The extra INCA changes each pass even though its byte count is divisible by 256.
    const PAINT_LOOP: [u8; 14] = [
        0x8E, 0x20, 0, 0xA7, 0x80, 0x4C, 0x8C, 0x80, 0, 0x26, 0xF8, 0x4C, 0x20, 0xF2,
    ];
    install(machine, &PAINT_LOOP);
}
