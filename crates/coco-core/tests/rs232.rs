//! Deluxe RS-232 Program Pak integration coverage (`docs/plan-deluxe-rs232.md`
//! "Testing / acceptance"): the `$FF60-$FF7E` spare-window bus routing, the
//! loopback round-trip ("byte written to `$FF68` reappears at `$FF68` with
//! RDRF set"), the ACIA-IRQ → CART* → PIA1 CB1 → FIRQ chain, the no-regression
//! open-bus checks for other cartridges, the 4K CTS EPROM window, and the
//! MPI slot-select routing/limitation.

use coco_core::acia6551::{command, status};
use coco_core::cart::{Cartridge, EmptySlot, IO_OPEN_BUS, MultiPak, RomPak, mpi};
use coco_core::config::{MachineVariant, MemorySize};
use coco_core::pia::cr;
use coco_core::rs232::DeluxeRs232;
use coco_core::{Machine, MachineConfig, SystemBus};
use mc6809::Bus;

const ACIA_DATA: u16 = 0xFF68;
const ACIA_STATUS: u16 = 0xFF69;
const ACIA_COMMAND: u16 = 0xFF6A;
const ACIA_CONTROL: u16 = 0xFF6B;

/// PIA1 port B addresses: data register (clears the CB1 flag on read) and
/// control register (CB1 edge/enable config).
const PIA1_PORTB_DATA: u16 = 0xFF22;
const PIA1_CRB: u16 = 0xFF23;

/// Control register value for 19200 baud (index 15), 8 data bits, 1 stop
/// bit — one frame is 10 bits ≈ 466 CPU cycles.
const CTL_19200_8N1: u8 = 0x0F;

/// Generous cycle budget for one full loopback round trip at 19200 baud:
/// TX frame (~466) + host-poll latency (128) + RX frame (~466), doubled.
const ROUND_TRIP_BUDGET: u32 = 2 * (466 + 128 + 466);

fn bus() -> SystemBus {
    SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    )
}

fn bus_with_pak() -> SystemBus {
    let mut bus = bus();
    bus.cart = Box::new(DeluxeRs232::new());
    bus
}

/// Advance the cartridge and the CART* interrupt seam the way
/// `Machine::run_cycles` does: tick in instruction-sized chunks, polling the
/// level between each.
fn run(bus: &mut SystemBus, cycles: u32) {
    let mut remaining = cycles;
    while remaining > 0 {
        let step = remaining.min(8);
        bus.cart.tick(step);
        bus.poll_cart_interrupt();
        remaining -= step;
    }
}

// ---- No-regression: the widened window stays open-bus for other carts ------

#[test]
fn empty_slot_reads_open_bus_at_acia_window() {
    let mut bus = bus();
    for addr in ACIA_DATA..=ACIA_CONTROL {
        assert_eq!(bus.read(addr), IO_OPEN_BUS, "addr {addr:#06X}");
        bus.write(addr, 0x55); // must be a no-op, not a panic
    }
}

#[test]
fn rom_pak_reads_open_bus_at_acia_window() {
    let mut bus = bus();
    bus.cart = Box::new(RomPak::from_bytes(&[0xA5; 0x2000], false).unwrap());
    for addr in ACIA_DATA..=ACIA_CONTROL {
        assert_eq!(bus.read(addr), IO_OPEN_BUS, "addr {addr:#06X}");
    }
}

/// The rest of the spare window ($FF60-$FF7E minus the ACIA's 4 registers)
/// is open bus even with the pak inserted — it decodes only $FF68-$FF6B.
#[test]
fn pak_leaves_rest_of_spare_window_open_bus() {
    let mut bus = bus_with_pak();
    for addr in (0xFF60..=0xFF7E).filter(|a| !(ACIA_DATA..=ACIA_CONTROL).contains(a)) {
        assert_eq!(bus.read(addr), IO_OPEN_BUS, "addr {addr:#06X}");
    }
}

// ---- Register routing through the bus --------------------------------------

#[test]
fn acia_registers_reachable_through_bus() {
    let mut bus = bus_with_pak();
    // Power-on: transmitter empty, no carrier.
    assert_ne!(bus.read(ACIA_STATUS) & status::TDRE, 0);
    bus.write(ACIA_COMMAND, command::DTR);
    assert_eq!(bus.read(ACIA_COMMAND), command::DTR);
    bus.write(ACIA_CONTROL, CTL_19200_8N1);
    assert_eq!(bus.read(ACIA_CONTROL), CTL_19200_8N1);
}

// ---- Acceptance: loopback round trip ----------------------------------------

#[test]
fn loopback_round_trip_sets_rdrf_and_returns_byte() {
    let mut bus = bus_with_pak();
    bus.write(ACIA_CONTROL, CTL_19200_8N1);
    bus.write(ACIA_COMMAND, command::DTR);
    bus.write(ACIA_DATA, 0x42);
    run(&mut bus, ROUND_TRIP_BUDGET);
    assert_ne!(
        bus.read(ACIA_STATUS) & status::RDRF,
        0,
        "byte should have looped back into RDR"
    );
    assert_eq!(bus.read(ACIA_DATA), 0x42);
    assert_eq!(
        bus.read(ACIA_STATUS) & status::RDRF,
        0,
        "RDR read clears RDRF"
    );
}

/// The full interrupt chain of the plan's acceptance test: rx-IRQ enabled,
/// loopback byte completes → ACIA IRQ asserts CART* → `poll_cart_interrupt`
/// drives PIA1 CB1 low (falling edge) → PIA1 FIRQ. Then unwinding it:
/// reading RDR + status drops the ACIA IRQ (CART* deasserts, CB1 returns
/// high — a non-selected edge), and reading PIA1's port B data register
/// clears the latched CB1 flag, releasing FIRQ.
#[test]
fn rx_irq_fires_firq_via_pia1_cb1() {
    let mut bus = bus_with_pak();
    // PIA1 CRB: CB1 interrupt enabled, falling edge (bit1=0) — the direction
    // CART* assertion drives.
    bus.write(PIA1_CRB, cr::C1_IRQ_ENABLE | cr::DDR_ACCESS);
    bus.write(ACIA_CONTROL, CTL_19200_8N1);
    // DTR enabled; command bit 1 clear = rx-IRQ enabled.
    bus.write(ACIA_COMMAND, command::DTR);
    assert!(!bus.firq_asserted());

    bus.write(ACIA_DATA, 0x99);
    run(&mut bus, ROUND_TRIP_BUDGET);
    assert!(bus.firq_asserted(), "RDRF with rx-IRQ enabled must reach FIRQ");

    // Unwind: RDR read clears RDRF, status read clears the ACIA IRQ output.
    assert_eq!(bus.read(ACIA_DATA), 0x99);
    bus.read(ACIA_STATUS);
    run(&mut bus, 16); // let the seam observe CART* deasserting
    assert!(
        bus.firq_asserted(),
        "PIA1 CB1 flag is latched until the data register is read"
    );
    bus.read(PIA1_PORTB_DATA);
    assert!(!bus.firq_asserted());
}

/// Same chain driven through the real `Machine::run_cycles` loop (a synthetic
/// all-zero ROM — the CPU executes harmless `NEG <$00` in page-0 RAM), proving
/// the per-instruction `poll_cart_interrupt` wiring in `lib.rs`, not just the
/// bus seam the `run` helper above emulates.
#[test]
fn machine_loop_polls_cart_interrupt() {
    let mut machine = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    machine.bus.cart = Box::new(DeluxeRs232::new());
    machine.bus.write(PIA1_CRB, cr::C1_IRQ_ENABLE | cr::DDR_ACCESS);
    machine.bus.write(ACIA_CONTROL, CTL_19200_8N1);
    machine.bus.write(ACIA_COMMAND, command::DTR);
    machine.bus.write(ACIA_DATA, 0x5A);
    machine.run_field();
    assert!(
        machine.bus.pia1.irq(),
        "run_cycles must convert the ACIA IRQ level into a PIA1 CB1 edge"
    );
}

/// End-to-end through a real socket: a host client connected to the pak's
/// TCP endpoint sees what the CPU writes to `$FF68`, and bytes it sends come
/// back out of `$FF68` — the plan's "drives a host endpoint visibly"
/// acceptance, with TCP standing in for the PTY.
#[test]
fn tcp_endpoint_round_trip_through_the_bus() {
    use std::io::{Read, Write};

    let endpoint =
        coco_core::serial::TcpEndpoint::bind("127.0.0.1:0").expect("bind an OS-assigned port");
    let addr = endpoint.local_addr().expect("bound address");
    let mut pak = DeluxeRs232::new();
    pak.set_endpoint(Box::new(endpoint));
    let mut bus = bus();
    bus.cart = Box::new(pak);

    let mut client = std::net::TcpStream::connect(addr).expect("connect to the pak");
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .expect("set read timeout");

    bus.write(ACIA_CONTROL, CTL_19200_8N1);
    bus.write(ACIA_COMMAND, command::DTR);

    // CPU -> host: the accept happens inside the pak's throttled host poll,
    // so keep ticking while the client waits for the byte.
    bus.write(ACIA_DATA, b'H');
    run(&mut bus, ROUND_TRIP_BUDGET);
    let mut byte = [0u8; 1];
    client.read_exact(&mut byte).expect("host sees the TX byte");
    assert_eq!(byte[0], b'H');

    // Host -> CPU: poll with a bounded retry loop — kernel socket delivery
    // isn't synchronous with our tick loop.
    client.write_all(b"K").expect("send a byte to the pak");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while bus.read(ACIA_STATUS) & status::RDRF == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "byte from the host never reached RDR"
        );
        run(&mut bus, ROUND_TRIP_BUDGET);
    }
    assert_eq!(bus.read(ACIA_DATA), b'K');
}

// ---- CTS EPROM window --------------------------------------------------------

#[test]
fn eprom_window_decodes_12_bits_and_wraps() {
    let mut pak = DeluxeRs232::new();
    // ROM-less pak: CTS reads answer open-bus $00.
    assert_eq!(pak.rom_read(0xC000), 0x00);

    let image: Vec<u8> = (0..0x1000u32).map(|i| (i % 251) as u8).collect();
    pak.set_eprom(&image);
    assert_eq!(pak.rom_read(0xC000), image[0]);
    assert_eq!(pak.rom_read(0xC123), image[0x123]);
    // Only 12 address bits decode (MAME cts_read `offset & 0x0fff`): the
    // image repeats every 4K across the window.
    assert_eq!(pak.rom_read(0xD123), image[0x123]);
    assert_eq!(pak.rom_read(0xFDFF), image[0x0DFF]);
}

// ---- Multi-Pak Interface routing ---------------------------------------------

/// Through an MPI the ACIA stays reachable regardless of the slot select:
/// the 6551 decodes the full address bus in the `$FF60-$FF7E` extension
/// window, which the MPI does not switch (only SCS*/CTS*/CART* are
/// per-slot; address and data buses are common to every slot, so
/// `MultiPak::read/write` broadcast this range).
#[test]
fn mpi_extension_window_ignores_the_slot_select() {
    let mut bus = bus();
    let mut mp = MultiPak::new(0);
    mp.insert(0, Box::new(DeluxeRs232::new()));
    bus.cart = Box::new(mp);

    bus.write(ACIA_COMMAND, command::DTR);
    assert_eq!(bus.read(ACIA_COMMAND), command::DTR);

    // Move the select register to slot 2 (both SCS and CTS fields): the
    // pak in slot 0 still answers — the extension window is not switched.
    bus.write(0xFF7F, mpi::SWITCH_VALUES[2]);
    assert_eq!(bus.read(ACIA_COMMAND), command::DTR);
}

/// CART* through the MPI follows the CTS slot select, like `rom_read` and
/// the Q-tie (the three lines the MPI switches together).
#[test]
fn mpi_forwards_cart_interrupt_from_cts_slot_only() {
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

    let mut mp = MultiPak::new(0);
    mp.insert(2, Box::new(AssertingCart));
    mp.insert(3, Box::new(EmptySlot));
    mp.control_write(mpi::SWITCH_VALUES[2]);
    assert!(mp.cart_interrupt());
    mp.control_write(mpi::SWITCH_VALUES[3]);
    assert!(!mp.cart_interrupt());
}
