//! The whole emulated machine: CPU + bus wiring, the scanline-driven run
//! loop, video mode classification/rendering, and the audio grid bridge.
//! See `DESIGN.md` §1/§2b.

mod audio;
mod render;
mod run;

pub use render::ActiveRect;
mod video_mode;

use mc6809::{Bus, MC6809};
use serde::{Deserialize, Serialize};

use crate::config::MachineConfig;
use crate::{GIME, cart, gime_video, pia, sam, video};

/// Framebuffer geometry: the VDG 32×16 text display plus border (`DESIGN.md` §6).
const FB_WIDTH: u32 = video::FB_W as u32;
const FB_HEIGHT: u32 = video::FB_H as u32;
const BYTES_PER_PIXEL: usize = video::BYTES_PER_PIXEL;

/// GIME palette value for the legacy CoCo-compatible text border: black.
const TEXT_BORDER_COLOR: u8 = 0x00;

/// NTSC CPU clock at normal speed: the 28.636363 MHz crystal / 32 (MAME
/// `coco3.cpp`). The SAM R1 bit doubles it (crystal / 16, ~1.79 MHz).
/// `pub(crate)` (re-exported as `crate::CPU_HZ` in `lib.rs`) so other
/// modules can derive cycle counts from the real clock instead of
/// duplicating the value — `cassette.rs`'s `RECORD_IDLE_FINALIZE_CYCLES`.
pub(crate) const CPU_HZ: f64 = 894_886.0;

/// GIME timer input clocks per normal-speed CPU cycle with INIT1 TINS=1. The
/// fast timer clock is 3.579545 MHz (279.365 ns — hardware-measured; MAME
/// `gime.cpp`. SEB's "70 ns" is wrong), exactly 4× the 0.89 MHz CPU clock —
/// and 2× the double-speed CPU clock, since the timer runs off the fixed
/// video crystal and ignores the CPU rate. With TINS=0 the input is the
/// ~63.5 µs horizontal sync: one tick per scanline.
const FAST_TIMER_TICKS_PER_CPU_CYCLE: u32 = 4;

/// Cap on buffered audio grid samples (~8 fields); beyond this the buffer
/// resets rather than growing (headless runs never drain it).
const AUDIO_BUFFER_CAP: usize = 8 * 262 * crate::audio::OVERSAMPLE as usize;

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
    GIMEText,
    /// INIT0 COCO=0, $FF98 BP=1: GIME native graphics (HSCREEN), up to 640-wide with
    /// a variable-size buffer. TODO(`DESIGN.md` §6).
    GIMEGraphics,
}

/// The whole emulated machine.
///
/// The CPU is one field and everything else lives in `bus`, so `cpu.step(&mut bus)`
/// borrows two disjoint fields without `Rc`/`RefCell` (`DESIGN.md` §2b).
#[derive(Serialize, Deserialize)]
pub struct Machine {
    pub cpu: MC6809,
    pub bus: crate::SystemBus,
    pub config: MachineConfig,
    /// RGBA framebuffer for the active video field (`DESIGN.md` §6). Its size is
    /// mode-dependent: each renderer fills a native-size buffer and the frontend
    /// scales to fit (`video-output-architecture` Option A). Skipped: cheap to
    /// rebuild (it's just the render target), rebuilt to the legacy geometry by
    /// [`Machine::after_restore`].
    #[serde(skip)]
    pub framebuffer: Vec<u8>,
    #[serde(skip)]
    pub fb_width: u32,
    #[serde(skip)]
    pub fb_height: u32,
    /// Scratch buffer for the VDG graphics video-RAM snapshot
    /// (`render_coco_graphics`), reused every field instead of reallocating.
    /// Skipped: derived scratch, regrows on demand from the `Default` empty `Vec`.
    #[serde(skip)]
    graphics_scratch: Vec<u8>,
    /// Stereo speaker samples on the oversampled grid
    /// ([`crate::audio::OVERSAMPLE`] per scanline, ~62.9 kHz on NTSC), `[left,
    /// right]`. [`Machine::flush_line_audio`] appends; the frontend drains
    /// via [`Machine::take_audio`] and resamples to the host rate.
    /// Self-capping so headless use (tests, no audio sink) doesn't grow it
    /// unboundedly. Skipped: derived scratch, regrows on demand from the
    /// `Default` empty `Vec`.
    #[serde(skip)]
    audio_buffer: Vec<[f32; 2]>,
    /// `SystemBus::cycle_clock` at the start of the scanline being executed
    /// — the left edge of the audio grid [`Machine::flush_line_audio`]
    /// renders at the line's end.
    audio_line_start: u64,
    /// The latched audio-input state at that same line start (events since
    /// then live in `SystemBus::audio_events`).
    audio_line_inputs: crate::audio::AudioInputs,
    /// True when the previous [`Machine::step_cpu_unit`] call burned a HALT*
    /// cycle instead of stepping. The MC6809 recognizes interrupts only at
    /// instruction-end boundaries, so the first instruction after HALT*
    /// releases must execute before a pending NMI/IRQ/FIRQ is serviced (see
    /// [`Machine::step_cpu_unit`]).
    prev_halted: bool,
    /// Current scanline within the field, in `0..lines_per_field`. Was the
    /// `for line in 0..lines` counter local to the old `run_field`; hoisting it
    /// (with `line_cycles_spent`/`line_budget`) into the machine is what makes
    /// execution resumable at instruction granularity. 0 at construction and
    /// after every completed field.
    line: u32,
    /// CPU cycles executed in the current scanline so far. Was `spent` inside
    /// the old `run_cycles`. Reset to 0 at each scanline boundary; the
    /// invariant after any completed `step_instruction` is
    /// `line_cycles_spent < line_budget` (mid-line) or `== 0` (just crossed
    /// into a new line/field).
    line_cycles_spent: u32,
    /// This scanline's cycle budget, sampled once at the line's start (when
    /// `line_cycles_spent == 0`) exactly like the old `run_field` sampled
    /// `cycles_per_field()/lines` at the top of each loop iteration — so a
    /// mid-field speed poke only takes effect on the next line. Also the
    /// multiplier the end-of-line GIME timer tick uses. Meaningful only once a
    /// line has begun; initialized to 0.
    line_budget: u32,
    /// Per-field video scanout state (CoCo 3 only), latched at the top of each
    /// field like MAME `new_frame`: the legacy-vs-GIME switch, the video base,
    /// and the smooth-scroll seed. `None` until the first field's first
    /// scanline completes. GIME-native fields paint the canonical raster line
    /// by line through this ([`gime_video::paint_scanline`]); legacy fields
    /// keep the whole-frame snapshot path in [`Machine::render_field`].
    field_scan: Option<gime_video::FieldScan>,
}

/// What a single [`Machine::step_instruction`] advanced. Both fields are
/// reported because one call can retire an instruction (or burn a HALT* cycle)
/// AND cross a scanline/field boundary in the same step — the debugger's
/// `run_until` needs the CPU action for its trace ring and the field flag for
/// its stop condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepEvent {
    /// What the CPU did this step.
    pub kind: StepKind,
    /// True when this step completed a video field: the per-scanline trailer
    /// for the field's last line ran, `line` wrapped back to 0, and the
    /// framebuffer was rendered — exactly the point the old `run_field`
    /// returned.
    pub field_complete: bool,
}

/// The CPU action a [`Machine::step_instruction`] performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// One instruction retired, consuming `cycles` bus cycles.
    Instruction {
        /// Bus cycles the instruction consumed.
        cycles: u32,
    },
    /// One HALT* cycle was burned (the cartridge holds the bus low); the CPU
    /// did not advance and no instruction retired.
    HaltCycle,
}

impl Machine {
    pub fn new(config: MachineConfig, rom: Box<[u8]>) -> Self {
        let mut cpu = MC6809::new();
        let mut bus = crate::SystemBus::new(config.variant, config.memory, rom);
        // `None` (CoCo 1/2) leaves the GIME default; unused on those variants.
        if let Some(monitor) = config.monitor {
            bus.gime.monitor = monitor;
        }
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
            audio_line_start: 0,
            audio_line_inputs: crate::audio::AudioInputs::default(),
            prev_halted: false,
            line: 0,
            line_cycles_spent: 0,
            line_budget: 0,
            field_scan: None,
        }
    }

    /// Restore-time fixups for every `#[serde(skip)]` field after a snapshot
    /// round-trip. A latched `field_scan` must reallocate the canvas-sized framebuffer here, or the next painted line indexes out of bounds.
    pub fn after_restore(&mut self) {
        if self.field_scan.is_some() {
            self.framebuffer.resize(
                crate::raster::CANVAS_W * crate::raster::CANVAS_H * BYTES_PER_PIXEL,
                0,
            );
            self.fb_width = crate::raster::CANVAS_W as u32;
            self.fb_height = crate::raster::CANVAS_H as u32;
        } else {
            self.reset_legacy_fb();
        }
        self.bus.after_restore();
    }

    /// The CPU clock, for callers converting cycle counts to wall-clock time
    /// outside the run loop. Always the normal-speed clock, regardless of the transient GIME double-speed POKE.
    pub fn cpu_hz(&self) -> f64 {
        CPU_HZ
    }

    /// Execute one CPU instruction; returns cycles consumed.
    pub fn step(&mut self) -> u32 {
        self.cpu.step(&mut self.bus)
    }

    /// The scanline within the current field (`0..lines_per_field`) execution
    /// is currently parked at.
    pub fn current_scanline(&self) -> u32 {
        self.line
    }

    /// Write one byte through the CPU's logical address space, with full side
    /// effects (unlike [`crate::SystemBus::peek`], which is read-only).
    pub fn poke(&mut self, addr: u16, val: u8) {
        self.bus.write(addr, val);
    }

    /// Re-run the CPU reset sequence (re-fetches the reset vector). Does not
    /// clear RAM. Also resets the cartridge, since RESET* is shared with the CPU's.
    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
        self.bus.cart.reset();
    }

    /// Power the machine off and on: clears RAM and returns the GIME/PIAs to
    /// power-on state, so the ROM runs its full cold-start path. Cartridge and cassette stay in their slots.
    pub fn power_cycle(&mut self) {
        // Monitor type is which cable is plugged in, not GIME state — survives power cycle.
        let monitor = self.bus.gime.monitor;
        self.bus.ram.fill(0);
        self.bus.gime = GIME::new();
        self.bus.gime.monitor = monitor;
        self.bus.sam = sam::SAM::new();
        self.bus.pia0 = pia::MC6821::new();
        self.bus.pia1 = pia::MC6821::new();
        self.prev_halted = false;
        // Restart the field scan from the top — power-on is a fresh field.
        self.line = 0;
        self.line_cycles_spent = 0;
        self.line_budget = 0;
        // `reset()` first: the re-latch below reads the reset cartridge.
        self.reset();
        self.bus.reset_edge_history();
        self.reset_audio_grid();
    }

    /// Power-on: re-latch inputs, drop queued events and undrained samples,
    /// and start the next line at the (still monotonic) `cycle_clock`.
    fn reset_audio_grid(&mut self) {
        self.bus.reset_audio_latch();
        self.audio_buffer.clear();
        self.audio_line_start = self.bus.cycle_clock;
        self.audio_line_inputs = self.bus.audio_inputs;
    }

    /// Plug a cartridge into the expansion port. Does not reset the machine —
    /// call [`Machine::power_cycle`] afterwards for the cold-start autostart/DK-probe logic to run.
    pub fn insert_cartridge(&mut self, cart: impl Into<cart::Cart>) {
        self.bus.cart = cart.into();
    }

    /// Remove the cartridge, restoring the empty slot. As with
    /// [`Machine::insert_cartridge`], call [`Machine::reset`] afterwards.
    pub fn eject_cartridge(&mut self) {
        self.bus.cart = cart::Cart::default();
    }
}
