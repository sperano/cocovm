//! Speaker output path: DAC through the analog mux (SNDEN + SEL=00) and the
//! always-connected single-bit sound (`DESIGN.md` §7; Tandy Service Manual mux
//! table via MAME coco.cpp).

use coco_core::{Machine, MachineConfig, MemorySize, SystemBus};
use mc6809::Bus;

const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_DDRA: u16 = 0xFF20; // with control DDR-selected
const PIA1_DA: u16 = 0xFF20;
const PIA1_DDRB: u16 = 0xFF22;
const PIA1_DB: u16 = 0xFF22;
const PIA1_CRA: u16 = 0xFF21;
const PIA1_CRB: u16 = 0xFF23;
/// Control value: data register selected, C2 set/reset output low/high.
const CR_C2_LOW: u8 = 0x34;
const CR_C2_HIGH: u8 = 0x3C;
/// Control value selecting the DDR (bit 2 clear).
const CR_DDR: u8 = 0x30;

fn bus() -> SystemBus {
    let mut b = SystemBus::new(MemorySize::K512, vec![0u8; 32 * 1024].into_boxed_slice());
    // DDRs: PIA1 PA2-7 outputs (DAC), PB1 output (single-bit sound).
    b.write(PIA1_CRA, CR_DDR);
    b.write(PIA1_DDRA, 0xFC);
    b.write(PIA1_CRB, CR_DDR);
    b.write(PIA1_DDRB, 0x02);
    // Data access, mux selects low (SEL=00), sound disabled.
    b.write(PIA1_CRA, CR_C2_LOW);
    b.write(PIA1_CRB, CR_C2_LOW);
    b.write(PIA0_CRA, CR_C2_LOW);
    b.write(PIA0_CRB, CR_C2_LOW);
    b
}

#[test]
fn dac_reaches_speaker_only_with_snden_and_mux_zero() {
    let mut b = bus();
    b.write(PIA1_DA, 0xFC); // DAC full scale

    assert_eq!(b.sound_sample(), 0.0, "SNDEN low: silent");

    b.write(PIA1_CRB, CR_C2_HIGH); // SNDEN high
    let loud = b.sound_sample();
    assert!(loud > 0.5, "SNDEN + SEL=00 routes the DAC: {loud}");

    b.write(PIA0_CRA, CR_C2_HIGH); // SEL1 high -> mux 01 (cassette): silent
    assert_eq!(b.sound_sample(), 0.0, "mux away from DAC: silent");
}

#[test]
fn single_bit_sound_is_always_connected() {
    let mut b = bus();
    // SNDEN low, mux irrelevant: PB1 alone must reach the speaker.
    b.write(PIA1_DB, 0x02);
    assert!(b.sound_sample() > 0.0);
    b.write(PIA1_DB, 0x00);
    assert_eq!(b.sound_sample(), 0.0);
}

#[test]
fn machine_collects_one_sample_per_scanline() {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    m.bus.write(0x0000, 0x20); // BRA *
    m.bus.write(0x0001, 0xFE);
    m.run_field();
    let lines = m.config.video.lines_per_field() as usize;
    assert_eq!(m.take_audio().count(), lines);
    // Drained: the next field starts fresh.
    m.run_field();
    assert_eq!(m.take_audio().count(), lines);
}
