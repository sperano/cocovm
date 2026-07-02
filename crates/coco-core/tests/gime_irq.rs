//! GIME interrupt controller + 12-bit interval timer ($FF90 IEN/FEN, $FF92/$FF93
//! enable/status, $FF94/$FF95 timer). Semantics per SEB Unravelled II (Fig 14,
//! interrupt chapter) and MAME `gime.cpp` (reload offset, clear-on-disable).

use std::path::PathBuf;

use coco_core::gime::{init0, intr, TIMER_RELOAD_OFFSET};
use coco_core::{keyboard, Machine, MachineConfig, MemorySize, SystemBus};
use mc6809::Bus;

const IRQENR: u16 = 0xFF92;
const FIRQENR: u16 = 0xFF93;
const TIMER_MSB: u16 = 0xFF94;
const TIMER_LSB: u16 = 0xFF95;
const INIT0: u16 = 0xFF90;

fn bus() -> SystemBus {
    SystemBus::new(MemorySize::K512, vec![0u8; 32 * 1024].into_boxed_slice())
}

// ---- Enable/status register semantics -----------------------------------------

#[test]
fn enabled_source_latches_and_read_clears() {
    let mut b = bus();
    b.write(IRQENR, intr::VBORD);
    b.vsync();
    // Latched; the read returns it and resets the flags (SEB: "reading the
    // status register resets the interrupt flags").
    assert_eq!(b.read(IRQENR), intr::VBORD);
    assert_eq!(b.read(IRQENR), 0);
}

#[test]
fn disabled_source_does_not_latch() {
    let mut b = bus();
    // No enables written: sync edges must leave the status empty.
    b.vsync();
    b.hsync();
    assert_eq!(b.read(IRQENR), 0);
    assert_eq!(b.read(FIRQENR), 0);
}

#[test]
fn writing_zero_to_enable_bit_clears_pending() {
    // The hardware anomaly SEB documents: disabling an interrupt clears its
    // latched status, exactly like reading the register.
    let mut b = bus();
    b.write(IRQENR, intr::VBORD | intr::HBORD);
    b.vsync();
    b.hsync();
    b.write(IRQENR, intr::HBORD); // drop VBORD enable -> drops its status too
    assert_eq!(b.read(IRQENR), intr::HBORD);
}

#[test]
fn irq_line_requires_init0_ien() {
    let mut b = bus();
    b.write(IRQENR, intr::VBORD);
    b.vsync();
    // Latched but the master enable is off: the CPU line stays high.
    assert!(!b.irq_asserted());
    b.write(INIT0, init0::IEN);
    assert!(b.irq_asserted());
    b.read(IRQENR); // acknowledge
    assert!(!b.irq_asserted());
}

#[test]
fn firq_path_is_independent_of_irq_path() {
    let mut b = bus();
    b.write(INIT0, init0::FEN);
    b.write(FIRQENR, intr::VBORD);
    b.vsync();
    assert!(b.firq_asserted());
    assert!(!b.irq_asserted());
    assert_eq!(b.read(FIRQENR), intr::VBORD);
    assert!(!b.firq_asserted());
}

// ---- Interval timer ------------------------------------------------------------

#[test]
fn timer_write_restarts_with_reload_offset() {
    let mut b = bus();
    b.write(TIMER_LSB, 10);
    // 1986 GIME: the count runs value + 2 input clocks per period.
    assert_eq!(b.gime.timer_reload, 10);
    assert_eq!(b.gime.timer_count, 10 + TIMER_RELOAD_OFFSET);
    // MSB holds the high nibble only; writing it restarts the full 12-bit value.
    b.write(TIMER_MSB, 0xF2);
    assert_eq!(b.gime.timer_reload, 0x020A);
    assert_eq!(b.gime.timer_count, 0x020A + TIMER_RELOAD_OFFSET);
}

#[test]
fn timer_underflow_raises_tmr_toggles_blink_and_reloads() {
    let mut b = bus();
    b.write(INIT0, init0::IEN);
    b.write(IRQENR, intr::TMR);
    b.write(TIMER_LSB, 10);
    let period = u32::from(10 + TIMER_RELOAD_OFFSET);

    b.gime.tick_timer(period - 1);
    assert_eq!(b.read(IRQENR), 0, "no underflow yet");
    let blink_before = b.gime.blink_state;

    b.gime.tick_timer(1);
    assert!(b.irq_asserted());
    assert_eq!(b.read(IRQENR), intr::TMR);
    assert_ne!(b.gime.blink_state, blink_before);
    // Auto-reloaded and counting again.
    assert_eq!(b.gime.timer_count, 10 + TIMER_RELOAD_OFFSET);
}

#[test]
fn timer_ticks_spanning_multiple_periods_all_fire() {
    let mut b = bus();
    b.write(IRQENR, intr::TMR);
    b.write(TIMER_LSB, 1);
    let period = u32::from(1 + TIMER_RELOAD_OFFSET);
    let blink_before = b.gime.blink_state;
    // Two full periods in one tick batch: blink toggles twice (back to start).
    b.gime.tick_timer(period * 2);
    assert_eq!(b.gime.blink_state, blink_before);
    assert_eq!(b.read(IRQENR), intr::TMR);
}

#[test]
fn timer_value_zero_inhibits_countdown() {
    let mut b = bus();
    b.write(IRQENR, intr::TMR);
    b.write(TIMER_LSB, 0);
    b.gime.tick_timer(100_000);
    assert_eq!(b.read(IRQENR), 0);
    assert!(!b.gime.blink_state);
}

// ---- Keyboard (EI1) edge ------------------------------------------------------

#[test]
fn keyboard_interrupt_fires_on_falling_edge_only() {
    let mut b = bus();
    b.write(IRQENR, intr::EI1);
    b.pia0.b.output = 0x00; // strobe every column low, as SEB's setup does

    b.hsync();
    assert_eq!(b.read(IRQENR), 0, "no key held: no interrupt");

    b.keyboard.set(keyboard::ENTER, true);
    b.hsync();
    assert_eq!(b.read(IRQENR), intr::EI1, "key press pulls a row low");

    b.hsync();
    assert_eq!(b.read(IRQENR), 0, "held key is not a new edge");

    b.keyboard.set(keyboard::ENTER, false);
    b.hsync();
    b.keyboard.set(keyboard::ENTER, true);
    b.hsync();
    assert_eq!(b.read(IRQENR), intr::EI1, "release + press is a new edge");
}

// ---- Real-ROM integration ------------------------------------------------------

fn boot_machine() -> Machine {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    let rom = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice();
    Machine::new(MachineConfig::default(), rom)
}

#[test]
fn rom_programs_the_timer_and_blink_phase_toggles() {
    // The cold start stores $FF to $FF94/$FF95 ("SET THE TIMER TO $FFFF AND
    // START IT COUNTING", SEB listing) with TINS=0, so the 12-bit timer runs a
    // 4097-hsync period (~16 fields): the blink phase must toggle repeatedly
    // while BASIC sits at the prompt.
    const FIELDS: usize = 120;
    let mut m = boot_machine();
    let mut toggles = 0;
    let mut last = m.bus.gime.blink_state;
    for _ in 0..FIELDS {
        m.run_field();
        if m.bus.gime.blink_state != last {
            toggles += 1;
            last = m.bus.gime.blink_state;
        }
    }
    assert_ne!(m.bus.gime.timer_reload, 0, "boot never programmed the timer");
    assert!(
        (4..=12).contains(&toggles),
        "expected ~7 blink toggles in {FIELDS} fields, saw {toggles}"
    );
}
