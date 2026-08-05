//! Tandy Deluxe RS-232 Program Pak (26-2226): a 6551 ACIA at `$FF68-$FF6B`
//! plus a 4K EPROM in the CTS window, glued to a host [`SerialEndpoint`].
//! Facts from MAME-authoritative
//! (`src/devices/bus/coco/coco_rs232.cpp`): the ACIA decodes the full
//! address bus itself (the pak sits outside the SCS* window, reached via
//! the bus's `$FF60-$FF7E` spare-window routing), its `_IRQ` output drives
//! the CART* pin as a *level* ([`Cartridge::cart_interrupt`] — no Q-tie
//! autostart, unlike game paks), and the EPROM decodes 12 address bits
//! (`eprom[offset & 0x0fff]`, coco_rs232.cpp `cts_read`), so the image
//! wraps every 4K across the CTS window.
//!
//! The wire glue in [`DeluxeRS232::tick`] is the only place the chip model
//! ([`ACIA6551`]) and the host backend ([`SerialEndpoint`]) meet: completed
//! TX frames are forwarded to the endpoint, and the endpoint is polled for
//! RX bytes only when the receiver is between frames — a byte the ACIA
//! isn't ready for stays queued host-side (kernel socket/pty buffer),
//! which is this model's stand-in for the sender's own pacing.

use serde::{Deserialize, Serialize};

use crate::acia6551::ACIA6551;
use crate::cart::{Cartridge, IO_OPEN_BUS, ROM_OPEN_BUS};
use crate::serial::{Loopback, SerialEndpoint};

/// First address the pak's ACIA answers (register select = `addr & 3`).
pub const ACIA_BASE: u16 = 0xFF68;
/// Last ACIA address (4 registers: data, status/reset, command, control).
pub const ACIA_LAST: u16 = 0xFF6B;

/// EPROM size: 4K, address bits `addr & 0x0FFF` (MAME `cts_read`), so any
/// image is mirrored across the 16K+ CTS window.
pub const EPROM_LEN: usize = 0x1000;

/// CPU cycles between host-endpoint polls in [`DeluxeRS232::tick`] (~143 µs
/// at the 0.894886 MHz clock). Polling the endpoint can cost a syscall
/// (nonblocking socket read / `accept`), so it must not run per instruction;
/// this interval stays well under one serial frame even at the ACIA's top
/// rate (19200 baud ≈ 466 cycles/frame), so throughput is never poll-bound.
const HOST_POLL_INTERVAL: u32 = 128;

/// `#[serde(default = "...")]` for [`DeluxeRS232::endpoint`]: matches
/// [`DeluxeRS232::new`]'s own inert default.
fn default_endpoint() -> Box<dyn SerialEndpoint> {
    Box::new(Loopback::new())
}

/// The Deluxe RS-232 Program Pak as a cartridge-port device.
#[derive(Serialize, Deserialize)]
pub struct DeluxeRS232 {
    acia: ACIA6551,
    /// Skipped: a host backend (TCP, PTY, …) is a host resource with no
    /// serializable shape. Deserializes to a fresh [`Loopback`] via
    /// `default_endpoint` below; the frontend re-plugs a real backend after
    /// restore through [`DeluxeRS232::set_endpoint`]
    ///.
    #[serde(skip, default = "default_endpoint")]
    endpoint: Box<dyn SerialEndpoint>,
    /// 4K EPROM image (the BASIC `DOS`/terminal ROM), if one was provided —
    /// a real dump isn't required to use the serial port from OS-9 or from
    /// hand-written BASIC `PEEK`/`POKE` code, so the pak works ROM-less
    /// (CTS reads answer open-bus). Skipped: COPYRIGHTED ROM bytes never
    /// travel through a snapshot; `None` is the correct restored default
    /// until the frontend re-injects it via the existing
    /// [`DeluxeRS232::set_eprom`].
    #[serde(skip)]
    eprom: Option<Box<[u8]>>,
    /// Cycles since the endpoint was last polled (see [`HOST_POLL_INTERVAL`]).
    since_host_poll: u32,
    /// Total bytes forwarded to the endpoint, for UI activity display.
    tx_bytes: u64,
    /// Total bytes delivered from the endpoint to the ACIA, for UI display.
    rx_bytes: u64,
}

impl DeluxeRS232 {
    /// Build a pak with no EPROM and a [`Loopback`] endpoint — the inert
    /// default until the frontend plugs in a real backend via
    /// [`DeluxeRS232::set_endpoint`].
    pub fn new() -> Self {
        Self {
            acia: ACIA6551::new(),
            endpoint: Box::new(Loopback::new()),
            eprom: None,
            since_host_poll: 0,
            tx_bytes: 0,
            rx_bytes: 0,
        }
    }

    /// Install the 4K EPROM image. Short images are accepted and read
    /// zero-filled at the tail (a real dump is exactly [`EPROM_LEN`] bytes;
    /// anything longer is truncated — only 12 address bits are decoded).
    pub fn set_eprom(&mut self, bytes: &[u8]) {
        let mut image = vec![0u8; EPROM_LEN].into_boxed_slice();
        let n = bytes.len().min(EPROM_LEN);
        image[..n].copy_from_slice(&bytes[..n]);
        self.eprom = Some(image);
    }

    /// Swap the host backend (TCP, PTY, loopback). The ACIA's in-flight
    /// frame state is untouched — this is re-plugging the cable, not
    /// resetting the chip.
    pub fn set_endpoint(&mut self, endpoint: Box<dyn SerialEndpoint>) {
        self.endpoint = endpoint;
    }

    /// Bytes sent to the host endpoint so far (UI activity counter).
    pub fn tx_bytes(&self) -> u64 {
        self.tx_bytes
    }

    /// Bytes received from the host endpoint so far (UI activity counter).
    pub fn rx_bytes(&self) -> u64 {
        self.rx_bytes
    }

    /// Direct access to the ACIA, for tests and debugger probes.
    pub fn acia(&mut self) -> &mut ACIA6551 {
        &mut self.acia
    }
}

impl Default for DeluxeRS232 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cartridge for DeluxeRS232 {
    /// The pak decodes only `$FF68-$FF6B`; everything else that reaches the
    /// cartridge (the SCS window, the rest of the spare window) floats.
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            ACIA_BASE..=ACIA_LAST => self.acia.read((addr & 0x03) as u8),
            _ => IO_OPEN_BUS,
        }
    }

    fn write(&mut self, addr: u16, val: u8) {
        if let ACIA_BASE..=ACIA_LAST = addr {
            self.acia.write((addr & 0x03) as u8, val);
        }
    }

    /// CTS EPROM window: 12 address bits, image mirrored across the window
    /// (MAME `cts_read`: `eprom[offset & 0x0fff]`).
    fn rom_read(&mut self, addr: u16) -> u8 {
        match &self.eprom {
            Some(image) => image[(addr & (EPROM_LEN as u16 - 1)) as usize],
            None => ROM_OPEN_BUS,
        }
    }

    /// ACIA `_IRQ` → CART* as a level (plan "Prerequisite B"); the bus
    /// converts transitions into the PIA1 CB1 edge / GIME EI0 raise.
    fn cart_interrupt(&mut self) -> bool {
        self.acia.irq_asserted()
    }

    fn tick(&mut self, cycles: u32) {
        self.acia.tick(cycles);
        // TX side: frames complete rarely (at most once per serial frame),
        // and checking is a cheap in-memory read, so no throttle here.
        while let Some(b) = self.acia.take_tx_byte() {
            self.endpoint.tx(b);
            self.tx_bytes += 1;
        }
        // RX side + modem lines: endpoint polls can cost syscalls, so run
        // them on the HOST_POLL_INTERVAL cadence.
        self.since_host_poll += cycles;
        if self.since_host_poll < HOST_POLL_INTERVAL {
            return;
        }
        self.since_host_poll = 0;
        // The endpoint trait models one "is anything there" line; feed it
        // to both DCD and DSR — the pak has no independent DSR source.
        let carrier = self.endpoint.dcd();
        self.acia.set_dcd(carrier);
        self.acia.set_dsr(carrier);
        // Pull a host byte only when the receiver is between frames; the
        // rest stays queued host-side (module doc comment).
        if self.acia.rx_ready()
            && let Some(b) = self.endpoint.poll_rx()
        {
            self.acia.receive_byte(b);
            self.rx_bytes += 1;
        }
    }

    /// The expansion port's RESET* line reaches the ACIA's hardware-reset
    /// pin. Host endpoint and EPROM are untouched — pressing reset doesn't
    /// unplug the cable.
    fn reset(&mut self) {
        self.acia.hardware_reset();
    }
}
