//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod bus;
pub mod cart;
pub mod cassette;
pub mod cassette_wav;
pub mod config;
pub mod fdc;
mod font6847;
mod font_gime;
pub mod gime;
pub mod gime_video;
pub mod joystick;
pub mod keyboard;
pub mod pia;
pub mod video;
pub mod wd1773;

pub use bus::SystemBus;
pub use config::{MachineConfig, MemorySize, VideoStandard};
pub use gime::GIME;

use mc6809::{Bus, MC6809};

/// Framebuffer geometry: the VDG 32×16 text display plus border (`DESIGN.md` §6).
const FB_WIDTH: u32 = video::FB_W as u32;
const FB_HEIGHT: u32 = video::FB_H as u32;
const BYTES_PER_PIXEL: usize = video::BYTES_PER_PIXEL;

/// GIME palette value for the legacy CoCo-compatible text border: black.
const TEXT_BORDER_COLOR: u8 = 0x00;

/// NTSC CPU clock at normal speed: the 28.636363 MHz crystal / 32 (MAME
/// `coco3.cpp`). The SAM R1 bit doubles it (crystal / 16, ~1.79 MHz).
const CPU_HZ: f64 = 894_886.0;

/// GIME timer input clocks per normal-speed CPU cycle with INIT1 TINS=1. The
/// fast timer clock is 3.579545 MHz (279.365 ns — hardware-measured; MAME
/// `gime.cpp`. SEB's "70 ns" is wrong), exactly 4× the 0.89 MHz CPU clock —
/// and 2× the double-speed CPU clock, since the timer runs off the fixed
/// video crystal and ignores the CPU rate. With TINS=0 the input is the
/// ~63.5 µs horizontal sync: one tick per scanline.
const FAST_TIMER_TICKS_PER_CPU_CYCLE: u32 = 4;

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
    /// INIT0 COCO=1, VDG bitmap graphics (PMODE), selected by PIA1 $FF22 A/G.
    /// Same 256×192 active area as text; lower resolutions are pixel-doubled.
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
    /// RGBA framebuffer for the active video field (`DESIGN.md` §6). Its size is
    /// mode-dependent: each renderer fills a native-size buffer and the frontend
    /// scales to fit (`video-output-architecture` Option A).
    pub framebuffer: Vec<u8>,
    pub fb_width: u32,
    pub fb_height: u32,
    /// Scratch buffer for the VDG graphics video-RAM snapshot
    /// (`render_coco_graphics`), reused every field instead of reallocating.
    graphics_scratch: Vec<u8>,
    /// Speaker samples, one per scanline (~15.7 kHz — the horizontal rate).
    /// `run_field` appends; the frontend drains via [`Machine::take_audio`]
    /// and resamples to the host rate. Self-capping so headless use (tests,
    /// no audio sink) doesn't grow it unboundedly.
    audio_buffer: Vec<f32>,
    /// True when the previous `run_cycles` iteration burned a HALT* cycle
    /// instead of stepping. The MC6809 recognizes interrupts only at
    /// instruction-end boundaries, so the first instruction after HALT*
    /// releases must execute before a pending NMI/IRQ/FIRQ is serviced (see
    /// [`Machine::run_cycles`]).
    prev_halted: bool,
}

/// Cap on buffered audio samples (~8 fields); beyond this the buffer resets
/// rather than growing (headless runs never drain it).
const AUDIO_BUFFER_CAP: usize = 8 * 262;

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
            graphics_scratch: Vec::new(),
            audio_buffer: Vec::new(),
            prev_halted: false,
        }
    }

    /// Drain the speaker samples accumulated since the last call (one per
    /// scanline, i.e. lines-per-field × field-rate ≈ 15.7 kHz).
    pub fn take_audio(&mut self) -> std::vec::Drain<'_, f32> {
        self.audio_buffer.drain(..)
    }

    /// The audio sample rate matching [`Machine::take_audio`]'s stream.
    pub fn audio_sample_rate(&self) -> f64 {
        self.config.video.lines_per_field() as f64 * self.config.video.field_rate_hz()
    }

    /// The CPU clock (the private `CPU_HZ` constant above) for callers
    /// converting cycle counts to wall-clock time outside the run loop —
    /// e.g. `cassette_wav`'s WAV encode/decode, which times tape bit
    /// periods in CPU cycles the same way
    /// [`Cassette::tick`](cassette::Cassette::tick) does. Always the
    /// normal-speed clock: the transient GIME double-speed POKE
    /// (`self.bus.gime.cpu_fast`) doesn't apply to cassette I/O, which the
    /// stock ROM never runs at double speed.
    pub fn cpu_hz(&self) -> f64 {
        CPU_HZ
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

    /// Plug a cartridge into the expansion port. Real cartridges are only
    /// swapped machine-off, and the stock ROM's autostart/DK-probe logic only
    /// runs at cold start, so this does not reset the machine itself — call
    /// [`Machine::reset`] afterwards.
    pub fn insert_cartridge(&mut self, cart: Box<dyn cart::Cartridge>) {
        self.bus.cart = cart;
    }

    /// Remove the cartridge, restoring the empty slot. As with
    /// [`Machine::insert_cartridge`], call [`Machine::reset`] afterwards.
    pub fn eject_cartridge(&mut self) {
        self.bus.cart = Box::new(cart::EmptySlot);
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
        for _ in 0..lines {
            // Sampled per line so a mid-field speed poke takes effect promptly.
            let cycles_per_line = self.cycles_per_field() / lines;
            self.run_cycles(cycles_per_line);
            self.bus.hsync();
            // One speaker sample per scanline (~15.7 kHz), self-capping when
            // nothing drains it.
            if self.audio_buffer.len() >= AUDIO_BUFFER_CAP {
                self.audio_buffer.clear();
            }
            self.audio_buffer.push(self.bus.sound_sample());
            // GIME interval timer: TINS=1 counts the fixed 3.58 MHz clock — 4
            // ticks per normal-speed CPU cycle, 2 per double-speed cycle —
            // TINS=0 counts horizontal syncs (1 per line).
            let ticks = if self.bus.gime.timer_is_fast() {
                let per_cycle = FAST_TIMER_TICKS_PER_CPU_CYCLE
                    / if self.bus.gime.cpu_fast { 2 } else { 1 };
                cycles_per_line * per_cycle
            } else {
                1
            };
            self.bus.gime.tick_timer(ticks);
        }
        self.bus.vsync();
        self.render_field();
    }

    /// Step instructions until at least `budget` cycles elapse, delivering any
    /// pending interrupt before each instruction.
    ///
    /// The cartridge HALT* line has priority over everything (MC6809 pin
    /// behaviour): while a device holds it — the FD-502's sector-transfer
    /// handshake — the CPU sits at an instruction boundary burning cycles and
    /// pending interrupts wait. The cartridge is ticked either way so it can
    /// pace the very work (DRQ cadence) that releases the line.
    ///
    /// The MC6809 recognizes interrupts only at the *end* of an instruction, so
    /// the first instruction after HALT* releases must execute before any
    /// pending NMI/IRQ/FIRQ is serviced. Skipping this lets the completion NMI
    /// of an FD-502 sector read preempt the DSKCON copy loop's `STB ,X+` that
    /// stores the sector's final byte — dropping one byte per sector on load.
    fn run_cycles(&mut self, budget: u32) {
        let mut spent = 0u32;
        while spent < budget {
            let cycles = if self.bus.halt_asserted() {
                self.prev_halted = true;
                1
            } else {
                // Coming straight out of HALT, run one instruction before
                // acknowledging interrupts (they stay pending for next loop).
                if !self.prev_halted {
                    if self.bus.take_nmi() {
                        self.cpu.nmi(&mut self.bus);
                    }
                    self.service_interrupts();
                }
                self.prev_halted = false;
                self.cpu.step(&mut self.bus)
            };
            self.bus.cart.tick(cycles);
            self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
            spent += cycles;
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
        let hz = if self.bus.gime.cpu_fast { CPU_HZ * 2.0 } else { CPU_HZ };
        (hz / self.config.video.field_rate_hz()) as u32
    }

    /// Classify the current video mode from the GIME registers.
    ///
    /// The CoCo-compatible text-vs-graphics split (VDG mode bits live in the SAM /
    /// PIA, not modelled yet) always resolves to text for now, so at the BASIC
    /// prompt this returns [`VideoMode::CocoText`].
    fn video_mode(&self) -> VideoMode {
        let g = &self.bus.gime;
        if g.init0 & gime::init0::COCO != 0 {
            // PIA1 $FF22 bit 7 selects VDG graphics (PMODE) vs alphanumerics/semigraphics.
            if self.bus.pia1.b.output & video::VDG_AG != 0 {
                VideoMode::CocoGraphics
            } else {
                VideoMode::CocoText
            }
        } else if g.vmode & gime::vmode::BP != 0 {
            VideoMode::GimeGraphics
        } else {
            VideoMode::GimeText
        }
    }

    /// Render one video field into `framebuffer`, dispatching on the current mode.
    fn render_field(&mut self) {
        match self.video_mode() {
            VideoMode::CocoText => self.render_coco_text(),
            VideoMode::CocoGraphics => self.render_coco_graphics(),
            VideoMode::GimeText => {
                // Blink phase is toggled by the GIME interval timer, which
                // BASIC programs at hi-res text setup (SEB Unravelled II).
                let blink_on = self.bus.gime.blink_state;
                let (w, h) = gime_video::render_text(
                    &self.bus.gime,
                    &self.bus.ram,
                    blink_on,
                    &mut self.framebuffer,
                );
                self.fb_width = w as u32;
                self.fb_height = h as u32;
            }
            VideoMode::GimeGraphics => {
                let (w, h) = gime_video::render_graphics(
                    &self.bus.gime,
                    &self.bus.ram,
                    &mut self.framebuffer,
                );
                self.fb_width = w as u32;
                self.fb_height = h as u32;
            }
        }
    }

    /// Render the legacy CoCo-compatible 32×16 text screen (`DESIGN.md` §6).
    fn render_coco_text(&mut self) {
        self.reset_legacy_fb();
        // Snapshot the text screen through the bus (honours the MMU) from the SAM
        // page-register base (the ROM programs $0400; CLS n / double-buffering
        // move it), then render.
        // TODO: per-scanline scanout straight from RAM (`DESIGN.md` §2b/§6).
        let base = self.bus.gime.sam_display_base();
        let mut screen = [0u8; video::SCREEN_LEN];
        for (i, cell) in screen.iter_mut().enumerate() {
            *cell = self.bus.read(base.wrapping_add(i as u16));
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

    /// Render a VDG bitmap graphics (PMODE) field (`DESIGN.md` §6).
    ///
    /// The mode/colour set come from PIA1 $FF22 and the display base from the SAM
    /// page register. Video RAM is read through the bus (honours the MMU) from that
    /// base — the same low-64K simplification as `render_coco_text`.
    fn render_coco_graphics(&mut self) {
        self.reset_legacy_fb();
        let ff22 = self.bus.pia1.b.output;
        let mode = video::decode_vdg_graphics(ff22, self.bus.gime.sam_video);
        let css = usize::from(ff22 & video::VDG_CSS != 0);
        let indices = video::vdg_palette_indices(mode.bpp, css);
        let mut colors = [[0u8; 4]; video::MAX_VDG_COLORS];
        for (slot, &reg) in colors.iter_mut().zip(indices) {
            *slot = GIME::rgb_color(self.bus.gime.palette[reg]);
        }

        let base = self.bus.gime.sam_display_base();
        self.graphics_scratch.resize(mode.bytes_per_row * mode.rows, 0);
        for (i, byte) in self.graphics_scratch.iter_mut().enumerate() {
            *byte = self.bus.read(base.wrapping_add(i as u16));
        }

        let border = GIME::rgb_color(TEXT_BORDER_COLOR);
        let colors = &colors[..indices.len()];
        video::render_graphics(&self.graphics_scratch, &mode, colors, border, &mut self.framebuffer);
    }

    /// Restore the fixed legacy-mode framebuffer geometry after a GIME-native
    /// mode may have resized it (e.g. WIDTH 32 back from WIDTH 80).
    fn reset_legacy_fb(&mut self) {
        self.framebuffer
            .resize((FB_WIDTH * FB_HEIGHT) as usize * BYTES_PER_PIXEL, 0);
        self.fb_width = FB_WIDTH;
        self.fb_height = FB_HEIGHT;
    }
}
