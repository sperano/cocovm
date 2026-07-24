//! PIA0 Cx1 edge-gating fidelity: CA1/CB1 flags must only latch on the
//! control-register-selected edge (`pia::cr::C1_EDGE_HIGH`), and the field
//! sync (CB1/VBORD) must land at its real mid-field scanlines
//! (`config::VideoStandard::fs_falling_line`/`fs_rising_line`), not at the
//! end of the field. Semantics verified against MAME `6821pia.cpp`
//! (`c1_low_to_high`/`c1_high_to_low`) and `gime.cpp`/`mc6847.cpp` (field-sync
//! scanline derivation).

use coco_core::pia::cr;
use coco_core::{MachineVariant, MemorySize, SystemBus, VideoStandard};
use mc6809::Bus;

const PIA0_CRA: u16 = 0xFF01;
const PIA0_CRB: u16 = 0xFF03;

fn bus() -> SystemBus {
    SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

// ---- Edge gating (bus-level, via hsync's CA1) ----------------------------------

#[test]
fn falling_edge_selected_port_flags_only_on_high_to_low() {
    let mut b = bus();
    // Default CRA ($FF01=0): C1_EDGE_HIGH clear -> falling edge selected.
    assert_eq!(b.pia0.a.control & cr::C1_EDGE_HIGH, 0);
    b.hsync(); // emits set_c1(false) then set_c1(true): falling edge matches
    assert_ne!(b.pia0.a.control & cr::C1_FLAG, 0);
}

#[test]
fn rising_edge_selected_port_flags_only_on_low_to_high() {
    let mut b = bus();
    b.write(PIA0_CRA, cr::C1_EDGE_HIGH); // select low->high
    b.hsync(); // falling edge (no match) then rising edge (matches)
    assert_ne!(b.pia0.a.control & cr::C1_FLAG, 0);
}

// PiaPort-level edge gating (falling/rising-selected flags only on the
// matching transition, never on a repeated level) is covered directly by
// `pia::tests::{falling_edge_selected_flags_only_on_high_to_low,
// rising_edge_selected_flags_only_on_low_to_high, repeated_level_never_flags}`.

// ---- Field-sync scanline placement ---------------------------------------------

#[test]
fn cb1_falling_flag_first_appears_at_fs_falling_line_not_before() {
    let mut b = bus();
    // Default CRB ($FF03=0): falling edge selected, matching stock BASIC's
    // $34/$35 ROM setup.
    let falling_line = VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3);
    for _ in 0..falling_line {
        b.hsync(); // drives CA1 only; CB1 must stay untouched all field
        assert_eq!(
            b.pia0.b.control & cr::C1_FLAG,
            0,
            "CB1 flag must not appear before the field-sync falling edge"
        );
    }
    b.fs_falling();
    assert_ne!(
        b.pia0.b.control & cr::C1_FLAG,
        0,
        "CB1 flag must appear exactly at the falling-edge scanline"
    );
}

#[test]
fn cb1_rising_edge_selected_polls_high_at_fs_rising_line() {
    let mut b = bus();
    // COLOR3-style setup: select the rising edge on CB1. $FF03 is always the
    // control register (reg 3), unaffected by DDR_ACCESS, so its bit 7 can be
    // polled directly without disturbing port B's data/DDR access mode.
    b.write(PIA0_CRB, cr::C1_EDGE_HIGH);
    let falling_line = VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3);
    let rising_line = VideoStandard::NTSC.fs_rising_line(MachineVariant::Coco3);
    for line in 0..rising_line {
        b.hsync();
        if line == falling_line {
            b.fs_falling(); // wrong direction for this port: must not flag
        }
        assert_eq!(
            b.read(PIA0_CRB) & 0x80,
            0,
            "no flag before the rising-edge scanline"
        );
    }
    b.fs_rising();
    assert_ne!(
        b.read(PIA0_CRB) & 0x80,
        0,
        "flag must appear exactly at the rising-edge scanline"
    );
}
