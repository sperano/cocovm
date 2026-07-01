//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod bus;
pub mod cart;
pub mod config;
pub mod gime;
pub mod pia;

pub use bus::SystemBus;
pub use config::{MachineConfig, MemorySize, VideoStandard};
pub use gime::GIME;

use mc6809::MC6809;

/// Placeholder framebuffer geometry until GIME video geometry is wired
/// (`DESIGN.md` §6).
const FB_WIDTH: u32 = 640;
const FB_HEIGHT: u32 = 480;
const BYTES_PER_PIXEL: usize = 4;

/// Provisional NTSC CPU clock (~0.895 MHz). Unverified constant; see `DESIGN.md` §4.
const CPU_HZ: f64 = 894_886.0;

/// The whole emulated machine.
///
/// The CPU is one field and everything else lives in `bus`, so `cpu.step(&mut bus)`
/// borrows two disjoint fields without `Rc`/`RefCell` (`DESIGN.md` §2b).
pub struct Machine {
    pub cpu: MC6809,
    pub bus: SystemBus,
    pub config: MachineConfig,
    /// RGBA framebuffer for the active video field (`DESIGN.md` §6).
    pub framebuffer: Vec<u8>,
    pub fb_width: u32,
    pub fb_height: u32,
}

impl Machine {
    pub fn new(config: MachineConfig, rom: Box<[u8]>) -> Self {
        let mut cpu = MC6809::new();
        let mut bus = SystemBus::new(config.memory, rom);
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            config,
            framebuffer: vec![0u8; (FB_WIDTH * FB_HEIGHT) as usize * BYTES_PER_PIXEL],
            fb_width: FB_WIDTH,
            fb_height: FB_HEIGHT,
        }
    }

    /// Execute one CPU instruction; returns cycles consumed.
    pub fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus)
    }

    /// Run one video field's worth of emulation.
    ///
    /// Skeleton: steps the CPU for an approximate per-field cycle budget and fills
    /// the framebuffer with the border colour. Per-scanline render and HSYNC/VSYNC
    /// interrupts are TODO (`DESIGN.md` §4, §6).
    pub fn run_field(&mut self) {
        let budget = self.cycles_per_field();
        let mut spent = 0u32;
        while spent < budget {
            spent += self.step();
        }
        self.render_field();
    }

    fn cycles_per_field(&self) -> u32 {
        (CPU_HZ / self.config.video.field_rate_hz()) as u32
    }

    fn render_field(&mut self) {
        // TODO: real GIME scanout. Placeholder solid fill so the frontend has
        // something to display.
        const PLACEHOLDER: [u8; BYTES_PER_PIXEL] = [0x20, 0x20, 0x30, 0xFF];
        for px in self.framebuffer.chunks_exact_mut(BYTES_PER_PIXEL) {
            px.copy_from_slice(&PLACEHOLDER);
        }
    }
}
