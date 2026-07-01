//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod bus;
pub mod cart;
pub mod config;
mod font6847;
pub mod gime;
pub mod keyboard;
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

/// Which video path the GIME is currently driving; `render_field` dispatches on it.
///
/// Only [`VideoMode::CocoText`] is implemented today. The other variants are the
/// branch points for the graphics renderers to come (PMODE and HSCREEN). Each
/// renderer fills its own-size buffer and the frontend scales to fit — see the
/// `video-output-architecture` note (Option A). Option B (one canonical raster) is
/// the planned follow-up for per-scanline mode changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VideoMode {
    /// INIT0 COCO=1, VDG alphanumerics/semigraphics: the legacy 32×16 text screen.
    CocoText,
    /// INIT0 COCO=1, VDG bitmap graphics (PMODE). Same 256×192 active area as text.
    /// TODO(`DESIGN.md` §6): needs the SAM V0–V2 / PIA GM mode bits, not modelled yet.
    /// Unconstructed until `video_mode` learns to read those bits.
    #[allow(dead_code)]
    CocoGraphics,
    /// INIT0 COCO=0, $FF98 BP=0: GIME native hi-res text (40/80 columns). TODO.
    GimeText,
    /// INIT0 COCO=0, $FF98 BP=1: GIME native graphics (HSCREEN), up to 640-wide with
    /// a variable-size buffer. TODO(`DESIGN.md` §6).
    GimeGraphics,
}

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

    /// Classify the current video mode from the GIME registers.
    ///
    /// The CoCo-compatible text-vs-graphics split (VDG mode bits live in the SAM /
    /// PIA, not modelled yet) always resolves to text for now, so at the BASIC
    /// prompt this returns [`VideoMode::CocoText`].
    fn video_mode(&self) -> VideoMode {
        let g = &self.bus.gime;
        if g.init0 & gime::init0::COCO != 0 {
            // TODO: inspect SAM V0–V2 / PIA GM bits to select CocoGraphics (PMODE).
            VideoMode::CocoText
        } else if g.vmode & gime::vmode::BP != 0 {
            VideoMode::GimeGraphics
        } else {
            VideoMode::GimeText
        }
    }

    /// Render one video field into `framebuffer`, dispatching on the current mode.
    ///
    /// Unimplemented graphics modes fall back to the text renderer so the machine
    /// keeps producing a picture; each arm is the seam where a real renderer lands.
    fn render_field(&mut self) {
        match self.video_mode() {
            VideoMode::CocoText => self.render_coco_text(),
            // TODO(`DESIGN.md` §6): VDG bitmap graphics (PMODE), 256×192 active.
            VideoMode::CocoGraphics => self.render_coco_text(),
            // TODO: GIME native hi-res text (40/80 columns, GIME character generator).
            VideoMode::GimeText => self.render_coco_text(),
            // TODO(`DESIGN.md` §6): GIME native graphics (HSCREEN); resize the buffer
            // and set fb_width/fb_height from the VRES bytes-per-row / LPF fields.
            VideoMode::GimeGraphics => self.render_coco_text(),
        }
    }

    /// Render the legacy CoCo-compatible 32×16 text screen (`DESIGN.md` §6).
    fn render_coco_text(&mut self) {
        // Snapshot the text screen through the bus (honours the MMU), then render.
        // TODO: per-scanline scanout straight from RAM (`DESIGN.md` §2b/§6).
        let mut screen = [0u8; video::SCREEN_LEN];
        for (i, cell) in screen.iter_mut().enumerate() {
            *cell = self.bus.read(TEXT_SCREEN_BASE + i as u16);
        }
        // Resolve the GIME palette registers the ROM programmed to RGBA. The
        // legacy CoCo-compatible text border is black (GIME `update_border`).
        let mut palette = [[0u8; 4]; video::PALETTE_LEN];
        for (i, entry) in palette.iter_mut().enumerate() {
            *entry = GIME::rgb_color(self.bus.gime.palette[i]);
        }
        let border = GIME::rgb_color(TEXT_BORDER_COLOR);
        video::render_text(&screen, &palette, border, &mut self.framebuffer);
    }
}
