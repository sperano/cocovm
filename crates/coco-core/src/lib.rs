//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod bus;
pub mod cart;
pub mod config;
mod font6847;
pub mod gime;
pub mod pia;
pub mod video;

pub use bus::SystemBus;
pub use config::{MachineConfig, MemorySize, VideoStandard};
pub use gime::GIME;

use mc6809::{Bus, MC6809};

/// Framebuffer geometry: the VDG 32×16 text display plus border (`DESIGN.md` §6).
const FB_WIDTH: u32 = video::FB_W as u32;
const FB_HEIGHT: u32 = video::FB_H as u32;
const BYTES_PER_PIXEL: usize = video::BYTES_PER_PIXEL;

/// Logical base of the CoCo-compatible text screen. Reading through the bus honours
/// the MMU mapping the CPU sees. TODO: track the true video base from the SAM
/// display-offset registers instead of assuming $0400 (`DESIGN.md` §6).
const TEXT_SCREEN_BASE: u16 = 0x0400;

/// GIME palette value for the legacy CoCo-compatible text border: black.
const TEXT_BORDER_COLOR: u8 = 0x00;

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

    /// Re-run the CPU reset sequence (re-fetches the reset vector from ROM). Does
    /// not clear RAM — a warm reset, like the CoCo's reset button.
    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
    }

    /// Run one video field's worth of emulation (`DESIGN.md` §4).
    ///
    /// Scanline-driven: each line runs a slice of CPU cycles then pulses the
    /// horizontal sync; the field ends by pulsing the field (vertical) sync. Both
    /// syncs are wired to PIA0 and drive the CPU IRQ, which is delivered between
    /// instructions — this is what breaks the stock ROM out of its idle loop and
    /// runs BASIC's housekeeping. Video scanout is filled at the end (`§6`).
    pub fn run_field(&mut self) {
        let lines = self.config.video.lines_per_field();
        let cycles_per_line = self.cycles_per_field() / lines;
        for _ in 0..lines {
            self.run_cycles(cycles_per_line);
            self.bus.hsync();
        }
        self.bus.vsync();
        self.render_field();
    }

    /// Step instructions until at least `budget` cycles elapse, delivering any
    /// pending interrupt before each instruction.
    fn run_cycles(&mut self, budget: u32) {
        let mut spent = 0u32;
        while spent < budget {
            self.service_interrupts();
            spent += self.cpu.step(&mut self.bus);
        }
    }

    /// Deliver pending FIRQ/IRQ to the CPU. The CPU itself honours the F/I masks
    /// and leaves a masked, still-asserted line pending for the next check.
    fn service_interrupts(&mut self) {
        if self.bus.firq_asserted() {
            self.cpu.firq(&mut self.bus);
        }
        if self.bus.irq_asserted() {
            self.cpu.irq(&mut self.bus);
        }
    }

    fn cycles_per_field(&self) -> u32 {
        (CPU_HZ / self.config.video.field_rate_hz()) as u32
    }

    fn render_field(&mut self) {
        // Snapshot the text screen through the bus (honours the MMU), then render.
        // TODO: per-scanline scanout straight from RAM (`DESIGN.md` §2b/§6).
        let mut screen = [0u8; video::SCREEN_LEN];
        for (i, cell) in screen.iter_mut().enumerate() {
            *cell = self.bus.read(TEXT_SCREEN_BASE + i as u16);
        }
        // Colours come from the GIME palette registers the ROM programmed. The
        // legacy CoCo-compatible text border is black (GIME `update_border`).
        let fg = GIME::rgb_color(self.bus.gime.palette[video::TEXT_FG_INDEX]);
        let bg = GIME::rgb_color(self.bus.gime.palette[video::TEXT_BG_INDEX]);
        let border = GIME::rgb_color(TEXT_BORDER_COLOR);
        video::render_text(&screen, fg, bg, border, &mut self.framebuffer);
    }
}
