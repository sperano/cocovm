//! `Machine::power_cycle` must reset derived state — interrupt edge history,
//! latched/queued/rendered audio — not just the GIME/SAM/PIAs. Runs a
//! zero-filled ROM (reset vector → $0000, harmless `NEG <$00`).

use coco_core::audio::OVERSAMPLE;
use coco_core::cart::{Cart, Cartridge, IO_OPEN_BUS};
use coco_core::gime::{init0, intr};
use coco_core::pia::cr;
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

const PIA0_DB: u16 = 0xFF02;
const PIA0_CRB: u16 = 0xFF03;
const PIA1_DA: u16 = 0xFF20;
const PIA1_CRA: u16 = 0xFF21;
const PIA1_CRB: u16 = 0xFF23;
const INIT0: u16 = 0xFF90;
const IRQENR: u16 = 0xFF92;
const FIRQENR: u16 = 0xFF93;
/// Control value selecting the DDR (bit 2 clear).
const CR_DDR: u8 = 0x30;
/// Data register selected, C2 set/reset output high (SNDEN on for PIA1 CRB).
const CR_C2_HIGH: u8 = 0x3C;
/// Data register selected, C2 set/reset output low.
const CR_C2_LOW: u8 = 0x34;
/// PIA1 DDRA: PA2–PA7 (the DAC) as outputs.
const DAC_OUTPUTS: u8 = 0xFC;
/// A loud DAC value: full scale on PA2–PA7.
const DAC_FULL_SCALE: u8 = 0xFC;
/// 'A' in the keyboard matrix: row 0, column 1.
const KEY_A: coco_core::keyboard::Pos = (0, 1);

/// Minimal cartridge whose CART* level is permanently asserted.
struct AssertingCart;
impl Cartridge for AssertingCart {
    fn read(&mut self, _addr: u16) -> u8 {
        IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn cart_interrupt(&mut self) -> bool {
        true
    }
}

fn machine() -> Machine {
    Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

/// Strobe every keyboard column (PIA0 PB0–PB7 outputs, all low).
fn strobe_all_columns(m: &mut Machine) {
    m.bus.write(PIA0_CRB, CR_DDR);
    m.bus.write(PIA0_DB, 0xFF);
    m.bus.write(PIA0_CRB, CR_C2_LOW);
    m.bus.write(PIA0_DB, 0x00);
}

/// Drive the DAC speaker path at full scale: PA2–PA7 outputs, SNDEN high.
fn drive_dac_full_scale(m: &mut Machine) {
    m.bus.write(PIA1_CRA, CR_DDR);
    m.bus.write(PIA1_DA, DAC_OUTPUTS);
    m.bus.write(PIA1_CRA, CR_C2_LOW);
    m.bus.write(PIA1_CRB, CR_C2_HIGH);
    m.bus.write(PIA1_DA, DAC_FULL_SCALE);
}

/// Finish the current scanline and return its `OVERSAMPLE` grid frames.
fn finish_line(m: &mut Machine) -> Vec<[f32; 2]> {
    let line = m.scanline();
    while m.scanline() == line {
        m.step_instruction();
    }
    let all: Vec<[f32; 2]> = m.take_audio().collect();
    assert!(
        all.len() >= OVERSAMPLE as usize,
        "at least one line flushed"
    );
    all[all.len() - OVERSAMPLE as usize..].to_vec()
}

#[test]
fn held_key_fires_gime_ei1_on_first_post_power_scanline() {
    let mut m = machine();
    m.bus.keyboard.set(KEY_A, true);
    strobe_all_columns(&mut m);
    m.bus.hsync(); // pre-power: row line sampled low
    m.power_cycle();
    // Key still held; post-power software strobes and enables EI1.
    strobe_all_columns(&mut m);
    m.bus.write(INIT0, init0::IEN);
    m.bus.write(IRQENR, intr::EI1);
    m.bus.hsync();
    assert!(
        m.bus.irq_asserted(),
        "an already-low row line must count as a falling edge after power-on"
    );
}

#[test]
fn asserted_cart_line_fires_pia1_cb1_and_gime_ei0_after_power_cycle() {
    let mut m = machine();
    m.insert_cartridge(Cart::custom(AssertingCart));
    m.bus.poll_cart_interrupt(); // pre-power: CART* sampled asserted
    m.power_cycle();
    m.bus.write(PIA1_CRB, cr::C1_IRQ_ENABLE | cr::DDR_ACCESS);
    m.bus.write(INIT0, init0::FEN);
    m.bus.write(FIRQENR, intr::EI0);
    m.bus.poll_cart_interrupt();
    assert!(
        m.bus.pia1.irq(),
        "PIA1 CB1 must see the still-asserted CART* as a fresh falling edge"
    );
    assert!(
        m.bus.firq_asserted(),
        "GIME EI0 must latch from the same first post-power sample"
    );
}

#[test]
fn stale_dac_latch_and_queued_events_do_not_leak_into_post_power_audio() {
    let mut silent = machine();
    let silence = finish_line(&mut silent);
    assert!(
        silence.iter().all(|s| *s == silence[0]),
        "a fresh machine's first line is flat"
    );

    let mut m = machine();
    drive_dac_full_scale(&mut m);
    m.power_cycle(); // mid-line, before any flush
    let first = finish_line(&mut m);
    assert_eq!(
        first, silence,
        "post-power grid must render the reset PIAs, not the pre-power DAC"
    );
}

#[test]
fn reprogramming_the_same_dac_value_after_power_cycle_is_audible() {
    let mut m = machine();
    drive_dac_full_scale(&mut m);
    finish_line(&mut m);
    m.power_cycle();
    let silent = finish_line(&mut m);
    // Same writes as pre-power: the latch was reset, so they are changes.
    drive_dac_full_scale(&mut m);
    let loud = finish_line(&mut m);
    assert_ne!(
        loud, silent,
        "the re-latched inputs must record the post-power DAC write as an event"
    );
}

#[test]
fn undrained_pre_power_samples_are_dropped() {
    let mut m = machine();
    drive_dac_full_scale(&mut m);
    m.run_field(); // renders samples nobody drains
    m.power_cycle();
    assert_eq!(
        m.take_audio().count(),
        0,
        "the powered-off machine's rendered sound must not be handed to the frontend"
    );
}
