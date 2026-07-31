//! Cartridge HALT*/NMI plumbing through the machine loop: while a cartridge
//! holds the HALT line the CPU must not execute (and interrupts must wait);
//! on release a pending NMI edge vectors exactly once. This is the substrate
//! the FD-502 disk controller's sector-transfer handshake runs on.

use std::cell::Cell;
use std::rc::Rc;

use coco_core::cart::{Cart, Cartridge};
use coco_core::{Machine, MachineConfig};
use mc6809::Bus;

const ROM_SIZE: usize = 32 * 1024;

/// The cartridge asserts HALT* this many cycles after reset — enough for the
/// boot code's `LDS` (which also arms NMI recognition) and a few loop
/// iterations, mimicking real code that is mid-run when the FDC halts it.
const HALT_FROM_CYCLES: u32 = 100;
/// ...and releases here: past the end of `run_field` #1 (~14.7k cycles) so
/// the whole remainder of field 1 is spent halted, and the release + NMI edge
/// land early in field 2.
const HALT_UNTIL_CYCLES: u32 = 15_000;
/// Iterations the counting loop could at most complete before the halt: the
/// loop body is 10 cycles, so ~100 pre-halt cycles allow ~10. A free-running
/// field would rack up ~1400 — the gap is what proves the halt stopped it.
const MAX_PRE_HALT_COUNT: u8 = 20;

/// RAM the test program counts loop iterations in.
const COUNTER_ADDR: u16 = 0x0400;
/// RAM the NMI handler writes its marker to.
const NMI_MARKER_ADDR: u16 = 0x0401;
const NMI_MARKER: u8 = 0xA5;

/// Synthetic 32K ROM: reset → `$8000` `LDS #$5EFF` (arms NMI recognition)
/// then `INC $0400; BRA *-3` (a visible-progress loop), NMI → `$8100`
/// `LDA #$A5; STA $0401; RTI`.
fn test_rom() -> Box<[u8]> {
    let mut rom = vec![0u8; ROM_SIZE];
    rom[0x0000..0x0009].copy_from_slice(&[0x10, 0xCE, 0x5E, 0xFF, 0x7C, 0x04, 0x00, 0x20, 0xFB]);
    rom[0x0100..0x0106].copy_from_slice(&[0x86, NMI_MARKER, 0xB7, 0x04, 0x01, 0x3B]);
    rom[0x7FFC..0x7FFE].copy_from_slice(&[0x81, 0x00]); // NMI vector → $8100
    rom[0x7FFE..0x8000].copy_from_slice(&[0x80, 0x00]); // RESET vector → $8000
    rom.into_boxed_slice()
}

/// A cartridge that asserts HALT* for the cycle window `halt_from..halt_until`,
/// raising one NMI edge at release. Shared `Cell`s let the test observe it.
struct HaltCart {
    ticks: Rc<Cell<u32>>,
    halt_from: u32,
    halt_until: u32,
    nmi_pending: Rc<Cell<bool>>,
}

impl Cartridge for HaltCart {
    fn read(&mut self, _addr: u16) -> u8 {
        coco_core::cart::IO_OPEN_BUS
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
    fn tick(&mut self, cycles: u32) {
        let before = self.ticks.get();
        self.ticks.set(before + cycles);
        // The NMI edge fires at the moment the halt releases.
        if before < self.halt_until && before + cycles >= self.halt_until {
            self.nmi_pending.set(true);
        }
    }
    fn halt_asserted(&self) -> bool {
        (self.halt_from..self.halt_until).contains(&self.ticks.get())
    }
    fn take_nmi(&mut self) -> bool {
        self.nmi_pending.replace(false)
    }
}

#[test]
fn halt_line_stops_the_cpu_and_nmi_fires_on_release() {
    let mut m = Machine::new(MachineConfig::default(), test_rom());
    let ticks = Rc::new(Cell::new(0));
    let nmi_pending = Rc::new(Cell::new(false));
    m.insert_cartridge(Cart::custom(HaltCart {
        ticks: Rc::clone(&ticks),
        halt_from: HALT_FROM_CYCLES,
        halt_until: HALT_UNTIL_CYCLES,
        nmi_pending: Rc::clone(&nmi_pending),
    }));
    m.reset();

    m.run_field();
    let pre_halt_count = m.bus.read(COUNTER_ADDR);
    assert!(pre_halt_count > 0, "CPU must run before the halt window");
    assert!(
        pre_halt_count <= MAX_PRE_HALT_COUNT,
        "CPU executed through the halt window: {pre_halt_count} loop iterations"
    );
    // Progress is measured in executed CPU cycles, not the loop counter — the
    // 8-bit counter can wrap back to its pre-halt value (it does: a full free
    // field runs an exact multiple of 256 iterations).
    let halted_field_cycles = m.cpu.cycles;
    assert!(
        halted_field_cycles < u64::from(2 * HALT_FROM_CYCLES),
        "CPU executed {halted_field_cycles} cycles through the halt window"
    );
    assert_eq!(m.bus.read(NMI_MARKER_ADDR), 0, "NMI vectored while halted");
    assert!(
        ticks.get() >= HALT_UNTIL_CYCLES - 1000,
        "cartridge must be ticked while the CPU is halted (got {} cycles)",
        ticks.get()
    );

    m.run_field();
    let resumed_cycles = m.cpu.cycles - halted_field_cycles;
    assert!(
        resumed_cycles > 10_000,
        "CPU did not resume after HALT* released ({resumed_cycles} cycles in field 2)"
    );
    assert_eq!(
        m.bus.read(NMI_MARKER_ADDR),
        NMI_MARKER,
        "pending NMI edge must vector once the halt releases"
    );
    assert!(!nmi_pending.get(), "NMI edge must be consumed");
}

#[test]
fn take_nmi_returns_true_at_most_once_per_edge() {
    let ticks = Rc::new(Cell::new(0));
    let nmi_pending = Rc::new(Cell::new(true));
    let mut cart = HaltCart {
        ticks,
        halt_from: 0,
        halt_until: 0,
        nmi_pending,
    };
    assert!(cart.take_nmi());
    assert!(!cart.take_nmi(), "a consumed edge must not repeat");
}
