//! The whole emulated machine: CPU + bus wiring, the scanline-driven run
//! loop, video mode classification/rendering, and the audio grid bridge.
//! See `DESIGN.md` §1/§2b.

mod artifact_phase;
mod audio;
mod render;
mod run;
mod text_cursor;

pub use render::ActiveRect;
pub use text_cursor::TextCursor;
mod video_mode;

use mc6809::{Bus, MC6809};
use serde::{Deserialize, Serialize};

use crate::config::MachineConfig;
use crate::{GIME, cart, gime_video, pia, raster, sam, video};

use artifact_phase::ArtifactPhaseState;

/// Framebuffer geometry: the canonical 640×240 raster every variant paints.
const FB_WIDTH: u32 = raster::CANVAS_W as u32;
const FB_HEIGHT: u32 = raster::CANVAS_H as u32;
const BYTES_PER_PIXEL: usize = video::BYTES_PER_PIXEL;
/// Bytes in the framebuffer: the one size both allocation sites use.
const FB_BYTES: usize = (FB_WIDTH * FB_HEIGHT) as usize * BYTES_PER_PIXEL;

/// GIME palette value for the legacy CoCo-compatible text border: black.
const TEXT_BORDER_COLOR: u8 = 0x00;

/// NTSC CPU clock at normal speed: the 28.636363 MHz crystal / 32 (MAME
/// `coco3.cpp`). The SAM R1 bit doubles it (crystal / 16, ~1.79 MHz).
/// `pub(crate)` (re-exported as `crate::CPU_HZ` in `lib.rs`) so other
/// modules can derive cycle counts from the real clock instead of
/// duplicating the value — `cassette.rs`'s `RECORD_IDLE_FINALIZE_CYCLES`.
pub(crate) const CPU_HZ: f64 = 894_886.0;
/// CPU clock under the double-speed poke (GIME R1 / SAM R1).
pub(crate) const FAST_CPU_HZ: f64 = CPU_HZ * 2.0;

/// GIME timer input clocks per normal-speed CPU cycle with INIT1 TINS=1. The
/// fast timer clock is 3.579545 MHz (279.365 ns — verified against MAME
/// `gime.cpp`; SEB's "70 ns" is wrong), exactly 4× the 0.89 MHz CPU clock —
/// and 2× the double-speed CPU clock, since the timer runs off the fixed
/// video crystal and ignores the CPU rate. With TINS=0 the input is the
/// ~63.5 µs horizontal sync: one tick per scanline.
const FAST_TIMER_TICKS_PER_CPU_CYCLE: u32 = 4;

/// Cap on buffered audio grid samples (~8 fields); beyond this the buffer
/// resets rather than growing (headless runs never drain it).
const AUDIO_BUFFER_CAP: usize = 8 * 262 * crate::audio::OVERSAMPLE as usize;

/// The video path that the GIME drives. `render_field` dispatches on this value.
///
/// Only [`VideoMode::CocoText`] is implemented. The other variants are the
/// branch points for planned PMODE and HSCREEN graphics renderers. Every
/// renderer paints the one canonical 640×240 raster (`video-output-architecture`
/// Option B), so the frontend never sees a mode-dependent size.
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
    /// RGBA framebuffer for the active video field (`DESIGN.md` §6): always the
    /// canonical 640×240 raster (`raster`), whatever the variant or mode. Skipped
    /// because the render target can be rebuilt; [`Machine::after_restore`]
    /// reallocates it.
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
    /// using [`Machine::take_audio`] and resamples to the host rate.
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
    /// MC14529 speaker-mux hold and crossfade state.
    #[serde(default)]
    audio_mux: crate::audio::AudioMux,
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
    /// `line_cycles_spent < line_budget` (mid-line) or `== 0` (entered a new
    /// line/field).
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
    /// Selected CoCo 1/2 NTSC artifact phase and its reset-to-reset random
    /// source. Serialized so resume preserves both the visible phase and the
    /// deterministic sequence used by later resets.
    #[serde(default)]
    artifact_phase: ArtifactPhaseState,
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
    /// One instruction retired. `cycles` includes any interrupt entry that
    /// immediately preceded the instruction.
    Instruction {
        /// Bus cycles the CPU unit consumed.
        cycles: u32,
    },
    /// One HALT* cycle was burned (the cartridge holds the bus low); the CPU
    /// did not advance and no instruction retired.
    HaltCycle,
}

impl Machine {
    pub fn new(config: MachineConfig, rom: Box<[u8]>) -> Self {
        Self::new_with_artifact_seed(config, rom, artifact_phase::fresh_seed())
    }

    /// Constructs a machine with a deterministic CoCo 1/2 artifact-phase
    /// source. Production callers use [`Self::new`]; tests can supply a seed
    /// to reproduce reset-time phase selections.
    pub(crate) fn new_with_artifact_seed(config: MachineConfig, rom: Box<[u8]>, seed: u64) -> Self {
        let mut cpu = MC6809::new();
        let mut bus = crate::SystemBus::new(config.variant, config.memory, rom);
        // `None` (CoCo 1/2) leaves the GIME default; unused on those variants.
        if let Some(monitor) = config.monitor {
            bus.gime.monitor = monitor;
        }
        cpu.reset(&mut bus);
        let artifact_phase = ArtifactPhaseState::new(seed, config.variant);
        Self {
            cpu,
            bus,
            config,
            framebuffer: vec![0u8; FB_BYTES],
            fb_width: FB_WIDTH,
            fb_height: FB_HEIGHT,
            graphics_scratch: Vec::new(),
            audio_buffer: Vec::new(),
            audio_line_start: 0,
            audio_line_inputs: crate::audio::AudioInputs::default(),
            audio_mux: crate::audio::AudioMux::new(),
            prev_halted: false,
            line: 0,
            line_cycles_spent: 0,
            line_budget: 0,
            field_scan: None,
            artifact_phase,
        }
    }

    /// Restore-time fixups for every `#[serde(skip)]` field after a snapshot
    /// round-trip. The skipped framebuffer must be reallocated to the canvas
    /// here, or the next painted line indexes out of bounds.
    pub fn after_restore(&mut self) {
        self.bus.variant = self.config.variant;
        self.framebuffer.resize(FB_BYTES, 0);
        self.fb_width = FB_WIDTH;
        self.fb_height = FB_HEIGHT;
        self.bus.after_restore();
    }

    /// The CPU clock, for callers converting cycle counts to wall-clock time
    /// outside the run loop. Always the normal-speed clock, regardless of
    /// the transient GIME double-speed POKE.
    pub fn cpu_hz(&self) -> f64 {
        CPU_HZ
    }

    /// Execute one CPU instruction using the raw MC6809 core only. Returns
    /// cycles consumed.
    ///
    /// Unlike [`Machine::step_instruction`], this bypasses every part of the
    /// machine execution boundary: HALT* (the cartridge HALT line has no
    /// effect), NMI/FIRQ/IRQ polling and servicing, cartridge/cassette/
    /// bitbanger peripheral ticks, `bus.cycle_clock`, scanline/field timing
    /// (hsync, the two field-sync edges, `render_scanline`/`render_field`),
    /// GIME interval-timer ticks, and audio sampling. A loop of this is a
    /// materially different (and, on the CoCo 3, non-interrupt-driven)
    /// emulator from one built on `step_instruction`.
    ///
    /// Only reach for this where that divergence is exactly what's wanted:
    /// deterministic pre-interrupt cold-start traces, and lockstep CPU-state
    /// comparisons that intentionally hold peripheral timing out of scope.
    /// Everything else — including anything that expects IRQ-driven
    /// behavior, correct audio/video, or real peripheral pacing — must use
    /// [`Machine::step_instruction`] or [`Machine::run_field`].
    pub fn step_cpu_raw(&mut self) -> u32 {
        self.cpu.step(&mut self.bus)
    }

    /// The scanline where execution is currently parked within the field
    /// (`0..lines_per_field`).
    pub fn current_scanline(&self) -> u32 {
        self.line
    }

    /// Largest `line_budget` a line could have sampled: the double-speed
    /// `cycles_per_field()` per line, whichever speed a snapshot was taken at.
    fn max_line_budget(&self) -> u32 {
        self.cycles_per_field_at(true) / self.config.video.lines_per_field()
    }

    /// Snapshot restore: checks `line`, `line_cycles_spent` and `line_budget`
    /// against the invariants documented on those fields.
    pub(crate) fn validate_restored_scheduler(&self) -> Result<(), String> {
        let lines = self.config.video.lines_per_field();
        if self.line >= lines {
            return Err(format!(
                "snapshot scanline {} is out of range for {lines} lines per field",
                self.line
            ));
        }
        let max_budget = self.max_line_budget();
        if self.line_budget > max_budget {
            return Err(format!(
                "snapshot line budget {} exceeds the maximum plausible {max_budget} cycles/line",
                self.line_budget
            ));
        }
        if self.line_cycles_spent != 0 && self.line_cycles_spent >= self.line_budget {
            return Err(format!(
                "snapshot line_cycles_spent {} is not less than line_budget {}",
                self.line_cycles_spent, self.line_budget
            ));
        }
        Ok(())
    }

    /// Write one byte through the CPU's logical address space, with full side
    /// effects (unlike [`crate::SystemBus::peek`], which is read-only).
    pub fn poke(&mut self, addr: u16, val: u8) {
        self.bus.write(addr, val);
    }

    /// Re-run the CPU reset sequence (re-fetches the reset vector). Does not
    /// clear RAM. Also resets the cartridge, since RESET* is shared with the
    /// CPU's.
    pub fn reset(&mut self) {
        if let Some(dw) = self.bus.drivewire.as_mut() {
            dw.reset_session();
        }
        self.artifact_phase.select_for_reset(self.config.variant);
        self.cpu.reset(&mut self.bus);
        self.bus.cart.reset();
    }

    /// The stable NTSC artifact phase selected for a CoCo 1/2. CoCo 3 output
    /// selects its live phase from BPI instead.
    pub fn ntsc_rg6_artifact_phase(&self) -> video::RG6ArtifactPhase {
        self.artifact_phase.selected()
    }

    /// Power the machine off and on: clears RAM and returns the GIME/PIAs to
    /// power-on state, so the ROM runs its full cold-start path. Cartridge
    /// and cassette stay in their slots.
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
        // `reset()` first: the following re-latch reads the reset cartridge.
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
        self.audio_mux = crate::audio::AudioMux::new();
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

#[cfg(test)]
#[path = "machine/artifact_phase_test.rs"]
mod artifact_phase_tests;
