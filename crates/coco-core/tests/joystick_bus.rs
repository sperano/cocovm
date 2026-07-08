//! Joystick read path through the bus: the CA2/CB2 mux selects, the DAC
//! comparator on PIA0 PA7, and fire buttons on the keyboard rows
//! (`DESIGN.md` §7; wiring verified against SEB Unravelled II + MAME coco.cpp).

use coco_core::joystick::{AXIS_X, AXIS_Y, LEFT, RIGHT};
use coco_core::{MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

const PIA0_PA: u16 = 0xFF00;
const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;
/// ROM-standard control value: data register selected, C2 set/reset output low.
const CR_C2_LOW: u8 = 0x34;
/// Same with the C2 output level high.
const CR_C2_HIGH: u8 = 0x3C;
const COMPARATOR: u8 = 0x80;

fn bus() -> SystemBus {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.write(PIA0_CRA, CR_C2_LOW);
    b.write(PIA0_CRB, CR_C2_LOW);
    b
}

fn set_dac(b: &mut SystemBus, value: u8) {
    // 6-bit DAC on PIA1 PA2-PA7; drive the output register directly.
    b.pia1.a.output = value << 2;
}

#[test]
fn comparator_tracks_dac_sweep_on_selected_pot() {
    let mut b = bus();
    b.joysticks.set_axis(RIGHT, AXIS_X, 40);

    set_dac(&mut b, 40);
    assert_ne!(b.read(PIA0_PA) & COMPARATOR, 0, "DAC == pot: comparator high");
    set_dac(&mut b, 41);
    assert_eq!(b.read(PIA0_PA) & COMPARATOR, 0, "DAC > pot: comparator low");
}

#[test]
fn mux_selects_follow_ca2_and_cb2() {
    let mut b = bus();
    b.joysticks.set_axis(RIGHT, AXIS_X, 10);
    b.joysticks.set_axis(RIGHT, AXIS_Y, 50);
    b.joysticks.set_axis(LEFT, AXIS_X, 50);
    set_dac(&mut b, 30);

    // SEL2:SEL1 = 00 -> right X (pot 10 < 30: low).
    assert_eq!(b.read(PIA0_PA) & COMPARATOR, 0);
    // SEL1 high (CA2) -> right Y (pot 50 >= 30: high).
    b.write(PIA0_CRA, CR_C2_HIGH);
    assert_ne!(b.read(PIA0_PA) & COMPARATOR, 0);
    // SEL2 high (CB2), SEL1 low -> left X (pot 50 >= 30: high).
    b.write(PIA0_CRA, CR_C2_LOW);
    b.write(PIA0_CRB, CR_C2_HIGH);
    assert_ne!(b.read(PIA0_PA) & COMPARATOR, 0);
}

#[test]
fn fire_buttons_pull_rows_low_regardless_of_strobe() {
    let mut b = bus();
    // Deselect every keyboard column (strobe all high), as BUTTON does.
    b.pia0.b.output = 0xFF;

    let idle = b.read(PIA0_PA);
    assert_eq!(idle & 0x0F, 0x0F, "no buttons: rows idle high");

    b.joysticks.set_button(RIGHT, 0, true);
    b.joysticks.set_button(LEFT, 1, true);
    let held = b.read(PIA0_PA);
    assert_eq!(held & 0x01, 0, "right button 1 pulls PA0");
    assert_eq!(held & 0x08, 0, "left button 2 pulls PA3");
    assert_eq!(held & 0x06, 0x06, "other rows stay high");
}
