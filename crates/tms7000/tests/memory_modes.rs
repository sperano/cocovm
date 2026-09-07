//! IOCNT0's memory-mode bits (SPND001C Table 3-5, Figure 3-16) decide
//! whether an address outside the register file/peripheral file/ROM
//! reaches `Bus::read_ext`/`write_ext` at all. Counts calls to those two
//! methods directly (rather than inferring reach from a value round-trip),
//! so "Not Available" (open bus, no `Bus` call) is distinguishable from
//! "external, but nothing happened to come back."

use tms7000::{Bus, Port, ROM_SIZE, Step, StepKind, TMS7040};

const IOCNT0: u8 = 0x00;

/// IOCNT0 bits 7:6 (SPND001C Figure 3-16): the memory-mode select.
const SINGLE_CHIP: u8 = 0x00;
const PERIPHERAL_EXPANSION: u8 = 0x40;
const FULL_EXPANSION: u8 = 0x80;
/// Not given a meaning by the manual; the implementation treats it as
/// Single-Chip (see `memory.rs`'s `MemoryMode`), which this table pins.
const UNDEFINED: u8 = 0xC0;

const MODES: [u8; 4] = [SINGLE_CHIP, PERIPHERAL_EXPANSION, FULL_EXPANSION, UNDEFINED];

/// Counts `read_ext`/`write_ext` calls instead of modeling real external
/// memory or ports.
#[derive(Default)]
struct CountingBus {
    ext_reads: u32,
    ext_writes: u32,
}

impl Bus for CountingBus {
    fn read_port(&mut self, _port: Port) -> u8 {
        0xFF
    }
    fn write_port(&mut self, _port: Port, _val: u8) {}
    fn read_ext(&mut self, _addr: u16) -> u8 {
        self.ext_reads += 1;
        0
    }
    fn write_ext(&mut self, _addr: u16, _val: u8) {
        self.ext_writes += 1;
    }
}

/// A powered-on chip with `code` at `$F000`, past its reset sequence, on a
/// fresh [`CountingBus`].
fn boot(code: &[u8]) -> (TMS7040, CountingBus) {
    let mut rom = vec![0; ROM_SIZE];
    rom[..code.len()].copy_from_slice(code);
    rom[ROM_SIZE - 2..].copy_from_slice(&0xF000u16.to_be_bytes());
    let mut cpu = TMS7040::new(&rom).expect("4 KB ROM");
    let mut bus = CountingBus::default();
    assert_eq!(
        cpu.step(&mut bus),
        Step {
            cycles: 17,
            kind: StepKind::Reset
        }
    );
    (cpu, bus)
}

/// `MOVP %mode,P0` (select the memory mode) then `LDA @>addr` or
/// `STA @>addr`; returns the `CountingBus` afterward.
fn run(mode: u8, ext_opcode: u8, addr: u16) -> CountingBus {
    let [hi, lo] = addr.to_be_bytes();
    let (mut cpu, mut bus) = boot(&[0xA2, mode, IOCNT0, ext_opcode, hi, lo]);
    cpu.step(&mut bus); // MOVP selects the mode
    cpu.step(&mut bus); // LDA/STA @>addr
    bus
}

const LDA: u8 = 0x8A;
const STA: u8 = 0x8B;

/// `(address, [reaches Bus under Single-Chip, Peripheral-Expansion,
/// Full-Expansion, Undefined])` — SPND001C Table 3-6 for the TMS70x0
/// family, plus Port D's Full-Expansion repurposing (`3.3.3`).
const CASES: &[(u16, [bool; 4])] = &[
    (0x007F, [false, false, false, false]), // register file: last byte
    (0x0080, [false, false, false, false]), // reserved: first byte
    (0x00FF, [false, false, false, false]), // reserved: last byte
    (0x0100, [false, false, false, false]), // on-chip PF: IOCNT0 itself
    (0x0108, [false, true, true, false]),   // CPORT: off-chip in both expansion modes
    (0x0109, [false, true, true, false]),   // CDDR: off-chip in both expansion modes
    (0x010A, [false, false, true, false]),  // DPORT: off-chip only in Full-Expansion
    (0x010B, [false, false, true, false]),  // DDDR: off-chip only in Full-Expansion
    (0x010C, [false, true, true, false]),   // peripheral expansion: first byte
    (0x01FF, [false, true, true, false]),   // peripheral expansion: last byte
    (0x0200, [false, false, true, false]),  // memory expansion: first byte
    (0xEFFF, [false, false, true, false]),  // memory expansion: last byte
    (0xF000, [false, false, false, false]), // ROM: first byte
];

#[test]
fn read_reaches_bus_exactly_when_the_mode_allows() {
    for &(addr, expected) in CASES {
        for (mode, want) in MODES.into_iter().zip(expected) {
            let bus = run(mode, LDA, addr);
            assert_eq!(
                bus.ext_reads > 0,
                want,
                "read {addr:#06X} under mode {mode:#04X}"
            );
        }
    }
}

#[test]
fn write_reaches_bus_exactly_when_the_mode_allows() {
    for &(addr, expected) in CASES {
        for (mode, want) in MODES.into_iter().zip(expected) {
            let bus = run(mode, STA, addr);
            assert_eq!(
                bus.ext_writes > 0,
                want,
                "write {addr:#06X} under mode {mode:#04X}"
            );
        }
    }
}

/// Reset re-initializes DDR/latch registers through the internal
/// peripheral-file path regardless of the memory mode reset found the chip
/// in (SPND001B 3.6.1): IOCNT0 clears to Single-Chip before those writes
/// land, so they never leak out as external bus cycles.
#[test]
fn reset_from_full_expansion_mode_does_not_leak_to_the_external_bus() {
    let (mut cpu, mut bus) = boot(&[0xA2, FULL_EXPANSION, IOCNT0]);
    cpu.step(&mut bus); // MOVP selects Full-Expansion
    let ext_calls_before = bus.ext_reads + bus.ext_writes;

    cpu.assert_reset();
    assert_eq!(
        cpu.step(&mut bus),
        Step {
            cycles: 17,
            kind: StepKind::Reset
        }
    );
    assert_eq!(
        bus.ext_reads + bus.ext_writes,
        ext_calls_before,
        "reset's port init must stay on-chip, not leak to Bus::read_ext/write_ext"
    );
    assert_eq!(cpu.port_ddr(Port::D), 0, "reset clears Port D's DDR");
}
