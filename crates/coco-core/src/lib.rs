//! `coco-core` — the headless CoCo 3 machine: bus, GIME, MMU, PIAs, timing.
//! No UI dependencies, so it can be unit-tested and boot a ROM without a window.
//! See `DESIGN.md` §1.

pub mod acia6551;
pub mod audio;
pub mod ay8913;
pub mod bitbanger;
pub mod bus;
pub mod cart;
pub mod cassette;
pub mod cassette_wav;
pub mod config;
pub mod debug;
pub mod dmp105;
mod dmp105_font;
pub mod drivewire;
pub mod fdc;
mod font6847;
mod font_gime;
pub mod gime;
pub mod gime_video;
pub mod joystick;
pub mod keyboard;
pub mod orch90;
pub mod pia;
pub mod printer;
pub mod raster;
pub mod rom_db;
pub mod rs232;
pub mod rtc;
pub mod sam;
pub mod serial;
pub mod sn76489;
pub mod ssc;
pub mod vhd;
pub mod video;
pub mod wd1773;

pub use bus::SystemBus;
pub use config::{MachineConfig, MachineVariant, MemorySize, VDGVariant, VideoStandard};
pub use gime::{GIME, MonitorType};

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
    /// Stereo speaker samples on the oversampled grid
    /// ([`audio::OVERSAMPLE`] per scanline, ~62.9 kHz on NTSC), `[left,
    /// right]`. [`Machine::flush_line_audio`] appends; the frontend drains
    /// via [`Machine::take_audio`] and resamples to the host rate.
    /// Self-capping so headless use (tests, no audio sink) doesn't grow it
    /// unboundedly.
    audio_buffer: Vec<[f32; 2]>,
    /// `SystemBus::cycle_clock` at the start of the scanline being executed
    /// — the left edge of the audio grid [`Machine::flush_line_audio`]
    /// renders at the line's end.
    audio_line_start: u64,
    /// The latched audio-input state at that same line start (events since
    /// then live in `SystemBus::audio_events`).
    audio_line_inputs: audio::AudioInputs,
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

/// Cap on buffered audio grid samples (~8 fields); beyond this the buffer
/// resets rather than growing (headless runs never drain it).
const AUDIO_BUFFER_CAP: usize = 8 * 262 * audio::OVERSAMPLE as usize;

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
        let mut bus = SystemBus::new(config.variant, config.memory, rom);
        bus.gime.monitor = config.monitor;
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
            audio_line_inputs: audio::AudioInputs::default(),
            prev_halted: false,
            line: 0,
            line_cycles_spent: 0,
            line_budget: 0,
            field_scan: None,
        }
    }

    /// Drain the speaker samples accumulated since the last call (one per
    /// scanline, i.e. lines-per-field × field-rate ≈ 15.7 kHz).
    pub fn take_audio(&mut self) -> std::vec::Drain<'_, [f32; 2]> {
        self.audio_buffer.drain(..)
    }

    /// The audio sample rate matching [`Machine::take_audio`]'s stream: the
    /// oversampled grid rate, [`audio::OVERSAMPLE`] × the scanline rate.
    pub fn audio_sample_rate(&self) -> f64 {
        self.line_rate() * f64::from(audio::OVERSAMPLE)
    }

    /// Scanlines per second (~15.7 kHz NTSC) — the audio grid's line clock.
    fn line_rate(&self) -> f64 {
        self.config.video.lines_per_field() as f64 * self.config.video.field_rate_hz()
    }

    /// Render the scanline that just executed to [`audio::OVERSAMPLE`]
    /// stereo grid samples (`docs/plan-audio-pipeline.md`).
    ///
    /// Latched inputs replay from the cycle-timestamped events the bus
    /// recorded during the line: each grid slot holds the state in effect
    /// at its start (a level change mid-slot lands on the next slot — grid
    /// resolution, the documented quantization). Generators are sampled
    /// per slot: the mux-gated cartridge input
    /// ([`cart::Cartridge::audio_sample`] — the AY drains a quarter-line
    /// of accumulated output) and the crystal PSG pair
    /// ([`cart::Cartridge::generator_sample`], wall-clock `dt` so the GIME
    /// double-speed poke can't retune them). The cassette level is sampled
    /// once per line — its 1200/2400 Hz square wave is far below even the
    /// line rate.
    fn flush_line_audio(&mut self) {
        let line_start = self.audio_line_start;
        let line_end = self.bus.cycle_clock;
        self.audio_line_start = line_end;
        // A HALT-free line spans `line_budget` cycles; keep the real span so
        // event timestamps land in the right slot even on odd lines.
        let span = line_end.saturating_sub(line_start).max(1);
        let slot_dt = 1.0 / self.audio_sample_rate();
        let cassette_bit = self.bus.cassette.playing() && self.bus.cassette.input_bit();

        let events = std::mem::take(&mut self.bus.audio_events);
        let mut inputs = self.audio_line_inputs;
        let mut cursor = 0;
        for k in 0..u64::from(audio::OVERSAMPLE) {
            let slot_start = line_start + span * k / u64::from(audio::OVERSAMPLE);
            while cursor < events.len() && events[cursor].cycle <= slot_start {
                inputs = events[cursor].inputs;
                cursor += 1;
            }
            let ay = self.bus.cart.audio_sample();
            let generators = self.bus.cart.generator_sample(slot_dt);
            self.audio_buffer
                .push(audio::mix(&inputs, cassette_bit, ay, generators));
        }
        // Events in the final slot's tail take effect from the next line's
        // first slot: the bus's current state is the next line's start state.
        self.audio_line_inputs = self.bus.audio_inputs;
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

    /// The scanline within the current field (`0..lines_per_field`) execution
    /// is currently parked at — the debugger's status bar and its "Step
    /// Scanline" control (`docs/plan-debugger.md` §3) are the only consumers;
    /// everything inside the crate uses the private `line` field directly.
    pub fn current_scanline(&self) -> u32 {
        self.line
    }

    /// Write one byte through the CPU's logical address space, with full
    /// side effects (unlike [`SystemBus::peek`], which is read-only by
    /// design) — the debugger's memory/register editors use this while the
    /// machine is paused, e.g. to poke a byte in the CoCo-logical memory
    /// view. Real hardware has no side-effect-free write; a debugger editing
    /// memory is expected to trip the same PIA/GIME register semantics a
    /// running program's own store would.
    pub fn poke(&mut self, addr: u16, val: u8) {
        self.bus.write(addr, val);
    }

    /// Re-run the CPU reset sequence (re-fetches the reset vector from ROM). Does
    /// not clear RAM — a warm reset, like the CoCo's reset button.
    ///
    /// Also resets the cartridge: the expansion port's RESET* line is shared
    /// with the CPU's, so a Multi-Pak Interface reloads its select register
    /// from the front-panel switch (and lifts any software override) on
    /// every reset, not just a cold power-on.
    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
        self.bus.cart.reset();
    }

    /// Power the machine off and on: clear RAM (so BASIC's warm-start magic
    /// is gone and the ROM runs its full cold-start path — including the DK
    /// probe that links Disk BASIC and the cartridge autostart check, both
    /// skipped on a warm reset) and return the GIME and PIAs to power-on
    /// state. The cartridge and cassette stay in their slots: this is what
    /// really happens when a cartridge is swapped on real hardware, which is
    /// only ever done machine-off.
    pub fn power_cycle(&mut self) {
        // Monitor type isn't GIME hardware state — it's which cable is
        // plugged into the back of the machine — so it survives a power
        // cycle same as it would on a real machine.
        let monitor = self.bus.gime.monitor;
        self.bus.ram.fill(0);
        self.bus.gime = GIME::new();
        self.bus.gime.monitor = monitor;
        self.bus.sam = sam::Sam::new();
        self.bus.pia0 = pia::MC6821::new();
        self.bus.pia1 = pia::MC6821::new();
        self.prev_halted = false;
        // Restart the field scan from the top — power-on is a fresh field.
        // (After any completed `run_field` these are already 0, so this only
        // matters if the machine was power-cycled mid partial step.)
        self.line = 0;
        self.line_cycles_spent = 0;
        self.line_budget = 0;
        self.reset();
    }

    /// Plug a cartridge into the expansion port. Real cartridges are only
    /// swapped machine-off, and the stock ROM's autostart/DK-probe logic only
    /// runs at cold start, so this does not reset the machine itself — call
    /// [`Machine::power_cycle`] afterwards (a warm [`Machine::reset`] skips
    /// the cold-start cartridge probes).
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
    /// horizontal sync. The field (vertical) sync is two separate edges at
    /// their own scanlines mid-field — not one pulse at the end of the loop
    /// — per [`config::VideoStandard::fs_falling_line`] /
    /// [`config::VideoStandard::fs_rising_line`]. All of these are wired to
    /// PIA0 and drive the CPU IRQ, which is delivered between instructions —
    /// this is what breaks the stock ROM out of its idle loop and runs
    /// BASIC's housekeeping. Video scanout is filled at the end (`§6`).
    pub fn run_field(&mut self) {
        // Resumable equivalent of the old nested scanline/cycle loops: drive
        // `step_instruction` from wherever the machine is parked until a field
        // completes. On a fresh machine (or right after any previous field —
        // both leave `line`/`line_cycles_spent` at 0) this is exactly one full
        // field from line 0, byte-for-byte identical to the pre-refactor loop.
        while !self.step_instruction().field_complete {}
    }

    /// Execute exactly one instruction (or one burned HALT* cycle) with full
    /// fidelity — peripheral ticks, NMI/FIRQ/IRQ servicing, and, when this step
    /// crosses the current scanline's cycle budget, the per-line trailer
    /// (hsync, the two field-sync edges at their lines, one audio sample, the
    /// GIME timer tick) and the field wrap (render + `field_complete`). This is
    /// the single primitive `run_field` and the debugger's `run_until` are both
    /// built on; the two together reproduce the old `run_field`/`run_cycles`
    /// nested loops one step at a time.
    ///
    /// Ordering is load-bearing and preserved exactly from the old code (see
    /// [`Machine::step_cpu_unit`] and [`Machine::end_of_line`]): the per-line
    /// trailer runs immediately after the instruction that pushes the line over
    /// budget, in the same call, before any instruction of the next line.
    pub fn step_instruction(&mut self) -> StepEvent {
        let lines = self.config.video.lines_per_field();
        loop {
            // Sample this line's budget once, at its start — same point the old
            // `run_field` sampled `cycles_per_field()/lines`.
            if self.line_cycles_spent == 0 {
                self.line_budget = self.cycles_per_field() / lines;
            }
            // Mirror `run_cycles`' `while spent < budget`: run one CPU unit if
            // the line still has budget. For real video timing `line_budget` is
            // always well above one instruction, so this branch always runs;
            // the `else` only guards a degenerate zero-budget line.
            if self.line_cycles_spent < self.line_budget {
                let (cycles, was_instruction) = self.step_cpu_unit();
                self.line_cycles_spent += cycles;
                let kind = if was_instruction {
                    StepKind::Instruction { cycles }
                } else {
                    StepKind::HaltCycle
                };
                // `run_cycles` exits when spent >= budget; `run_field` then runs
                // the per-line trailer. A single CPU unit (≤ ~20 cycles) can
                // cross at most one ~57-cycle line boundary, so one trailer
                // suffices.
                let field_complete = if self.line_cycles_spent >= self.line_budget {
                    let done = self.end_of_line();
                    self.line_cycles_spent = 0;
                    done
                } else {
                    false
                };
                return StepEvent { kind, field_complete };
            }
            // Degenerate zero-budget line (never reached for real timing): no
            // CPU unit to run — do the trailer and continue to the next line so
            // every call still makes forward progress.
            let field_complete = self.end_of_line();
            self.line_cycles_spent = 0;
            if field_complete {
                return StepEvent { kind: StepKind::HaltCycle, field_complete: true };
            }
        }
    }

    /// One iteration of the old `run_cycles` inner loop: burn a HALT* cycle or
    /// execute one instruction, then tick the per-cycle peripherals. Returns
    /// `(cycles, was_instruction)`.
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
    fn step_cpu_unit(&mut self) -> (u32, bool) {
        let (cycles, was_instruction) = if self.bus.halt_asserted() {
            self.prev_halted = true;
            (1, false)
        } else {
            // Coming straight out of HALT, run one instruction before
            // acknowledging interrupts (they stay pending for next loop).
            if !self.prev_halted {
                self.bus.poll_cart_interrupt();
                if self.bus.take_nmi() {
                    self.cpu.nmi(&mut self.bus);
                }
                self.service_interrupts();
            }
            self.prev_halted = false;
            (self.cpu.step(&mut self.bus), true)
        };
        self.bus.cart.tick(cycles);
        self.bus.cassette.tick(cycles, self.bus.pia1.a.c2_output());
        self.bus.bitbanger.tick(cycles, self.bus.pia1_tx_mark());
        self.bus.cycle_clock = self.bus.cycle_clock.wrapping_add(u64::from(cycles));
        (cycles, was_instruction)
    }

    /// The per-scanline trailer from the old `run_field` loop body, run after
    /// the current line's cycle budget is spent: horizontal sync, the two
    /// field-sync edges when `line` matches, one speaker sample, and the GIME
    /// interval-timer tick. Advances `line`; at the end of the field it wraps
    /// to 0, renders the framebuffer, and returns `true`.
    fn end_of_line(&mut self) -> bool {
        let lines = self.config.video.lines_per_field();
        let fs_falling_line = self.config.video.fs_falling_line(self.config.variant);
        let fs_rising_line = self.config.video.fs_rising_line(self.config.variant);
        self.bus.hsync();
        if self.line == fs_falling_line {
            self.bus.fs_falling();
        }
        if self.line == fs_rising_line {
            self.bus.fs_rising();
        }
        self.render_scanline();
        // Render this line's audio to the oversampled stereo grid,
        // self-capping when nothing drains it.
        if self.audio_buffer.len() >= AUDIO_BUFFER_CAP {
            self.audio_buffer.clear();
        }
        self.flush_line_audio();
        // GIME interval timer: TINS=1 counts the fixed 3.58 MHz clock — 4 ticks
        // per normal-speed CPU cycle, 2 per double-speed cycle — TINS=0 counts
        // horizontal syncs (1 per line). No such timer exists on the plain-SAM
        // path (CoCo 1/2) — the GIME stays completely inert there
        // (`docs/coco12-plan.md` Phase 4). `line_budget` is this line's sampled
        // cycle count — the old loop's `cycles_per_line`.
        if self.config.variant == MachineVariant::Coco3 {
            let ticks = if self.bus.gime.timer_is_fast() {
                let per_cycle =
                    FAST_TIMER_TICKS_PER_CPU_CYCLE / if self.bus.gime.cpu_fast { 2 } else { 1 };
                self.line_budget * per_cycle
            } else {
                1
            };
            self.bus.gime.tick_timer(ticks);
        }
        self.line += 1;
        if self.line >= lines {
            self.line = 0;
            self.render_field();
            true
        } else {
            false
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
        // Speed-poke source differs per variant: the GIME's own R1 latch on
        // CoCo 3, the plain SAM's R0|R1 strobes on CoCo 1/2
        // (`docs/coco12-plan.md` Phase 4; `Sam::cpu_fast`'s KNOWN GAP note).
        let cpu_fast = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.cpu_fast,
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.cpu_fast(),
        };
        let hz = if cpu_fast { CPU_HZ * 2.0 } else { CPU_HZ };
        (hz / self.config.video.field_rate_hz()) as u32
    }

    /// Classify the current video mode.
    ///
    /// CoCo 1/2 (no GIME) never has INIT0/$FF98 to consult: they always run the
    /// VDG-native path, chosen purely by PIA1 $FF22 bit 7 (A/G) —
    /// `docs/coco12-plan.md` Phase 3. CoCo 3 keeps its existing INIT0 COCO /
    /// $FF98 BP dispatch, unchanged.
    fn video_mode(&self) -> VideoMode {
        match self.config.variant {
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                if self.bus.pia1.b.output & video::VDG_AG != 0 {
                    VideoMode::CocoGraphics
                } else {
                    VideoMode::CocoText
                }
            }
            MachineVariant::Coco3 => {
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
        }
    }

    /// CoCo-compatible video/text base address, per variant: the GIME's own
    /// SAM-compat page register (CoCo 3, unchanged) or the primary SAM's F-bits
    /// (CoCo 1/2 — `docs/coco12-plan.md` Phase 3).
    fn legacy_display_base(&self) -> u16 {
        match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.sam_display_base(),
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.display_base() as u16,
        }
    }

    /// Resolve the 16-entry colour table the CoCo-compatible text/graphics
    /// renderers read from, per variant (`docs/coco12-plan.md` Phase 3):
    /// CoCo 3 snapshots the GIME palette registers (existing behaviour,
    /// unchanged); CoCo 1/2 has none, so it resolves the fixed VDG RGB table.
    /// `css` (PIA1 $FF22 bit 3) only matters for the fixed-VDG path — see
    /// [`video::ColorSource::resolve`].
    fn legacy_palette(&self, css: bool) -> [[u8; 4]; video::PALETTE_LEN] {
        match self.config.variant {
            MachineVariant::Coco3 => {
                let mut resolved = [[0u8; 4]; video::PALETTE_LEN];
                for (i, entry) in resolved.iter_mut().enumerate() {
                    *entry = self.bus.gime.color(self.bus.gime.palette[i]);
                }
                video::ColorSource::GimePalette(&resolved).resolve(css)
            }
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::ColorSource::VdgFixed.resolve(css)
            }
        }
    }

    /// Decode the current text screen to ASCII lines, whichever video mode is
    /// active — a debug/probe helper, not a renderer (`gime_video::text_lines`
    /// and `video::decode_alpha_char` do the actual decoding, shared with the
    /// real renderers so this can't drift from what's actually on screen):
    ///
    /// - CoCo-compatible mode (INIT0 COCO=1): the legacy VDG alphanumeric
    ///   screen at the SAM page base, decoded the same way `render_coco_text`
    ///   reads it (through the bus, honouring the MMU).
    /// - GIME hi-res text (INIT0 COCO=0, $FF98 BP=0): the GIME-native text
    ///   buffer at the vertical-offset registers' physical address.
    ///
    /// The two graphics modes (VDG PMODE, GIME HSCREEN) have no text buffer to
    /// decode; each returns one placeholder line naming the mode. Pair with
    /// [`Machine::video_mode_summary`] to tell a "genuinely blank screen" apart
    /// from "this is a graphics-mode screen with nothing to decode".
    pub fn text_screen_lines(&mut self) -> Vec<String> {
        match self.video_mode() {
            VideoMode::CocoText | VideoMode::CocoGraphics => {
                let base = self.legacy_display_base();
                (0..video::ROWS as u16)
                    .map(|row| {
                        (0..video::COLS as u16)
                            .map(|col| {
                                let addr = base.wrapping_add(row * video::COLS as u16 + col);
                                video::decode_alpha_char(self.bus.read(addr))
                            })
                            .collect()
                    })
                    .collect()
            }
            VideoMode::GimeText => gime_video::text_lines(&self.bus.gime, &self.bus.ram),
            VideoMode::GimeGraphics => {
                vec!["<no text buffer: GIME graphics mode (HSCREEN, $FF98 BP=1)>".to_string()]
            }
        }
    }

    /// One-line diagnostic summary of the current video mode and its text/video
    /// base address. Pairs with [`Machine::text_screen_lines`] to explain an
    /// unexpectedly blank or garbled dump — most commonly, the machine has
    /// switched to a graphics mode, which has no text buffer.
    pub fn video_mode_summary(&self) -> String {
        match self.video_mode() {
            VideoMode::CocoText => {
                format!(
                    "video mode: CoCo-compatible text, base=${:04X}",
                    self.legacy_display_base()
                )
            }
            VideoMode::CocoGraphics => {
                format!(
                    "video mode: CoCo-compatible graphics (PMODE), base=${:04X}",
                    self.legacy_display_base()
                )
            }
            VideoMode::GimeText => {
                format!(
                    "video mode: GIME hi-res text, base=${:06X}",
                    self.bus.gime.video_base()
                )
            }
            VideoMode::GimeGraphics => {
                format!(
                    "video mode: GIME graphics (HSCREEN), base=${:06X}",
                    self.bus.gime.video_base()
                )
            }
        }
    }

    /// Paint the current scanline of the canonical raster (Option B,
    /// `docs/plan-per-scanline-video.md`), called from [`Machine::end_of_line`]
    /// at every line so mid-frame register writes take effect on the next line.
    ///
    /// At line 0 the per-field register group is latched (MAME `new_frame`):
    /// the INIT0 COCO switch, the video base, and the smooth-scroll seed —
    /// one line-time later than MAME's field start, within the plan's
    /// line-granular contract. Only GIME-native fields (CoCo 3, COCO=0) paint
    /// here; legacy fields keep the whole-frame path in
    /// [`Machine::render_field`], and CoCo 1/2 has no GIME to latch at all.
    fn render_scanline(&mut self) {
        if self.config.variant != MachineVariant::Coco3 {
            return;
        }
        if self.line == 0 {
            let legacy = self.bus.gime.init0 & gime::init0::COCO != 0;
            self.field_scan = Some(gime_video::FieldScan::latch(&self.bus.gime, legacy));
            self.framebuffer
                .resize(raster::CANVAS_W * raster::CANVAS_H * BYTES_PER_PIXEL, 0);
            self.fb_width = raster::CANVAS_W as u32;
            self.fb_height = raster::CANVAS_H as u32;
        }
        let row = self.line as usize;
        let Some(scan) = self.field_scan.as_ref() else {
            return;
        };
        if row >= raster::CANVAS_H {
            return; // blanking lines 240..262
        }
        if scan.legacy {
            self.paint_legacy_scanline(row);
            return;
        }
        // Blink phase is toggled by the GIME interval timer, which BASIC
        // programs at hi-res text setup (SEB Unravelled II).
        let blink_on = self.bus.gime.blink_state;
        let scan = self.field_scan.as_mut().expect("checked Some above");
        gime_video::paint_scanline(
            &self.bus.gime,
            &self.bus.ram,
            scan,
            blink_on,
            row,
            &mut self.framebuffer,
        );
    }

    /// Paint one canvas row of a CoCo 3 legacy (VDG-compatible) field. Same
    /// per-line contract as the GIME-native painter — mode bits ($FF22, SAM
    /// V), palette/CSS, and the border are read live each line; the row
    /// pointer and glyph-row counter carry across lines — with the legacy
    /// data path: 16-bit logical fetches through the bus (honouring the MMU),
    /// like the whole-field renderers did. The border follows MAME
    /// `update_border`'s legacy rule ([`video::legacy_border_value`]), NOT
    /// fixed black: green/white for graphics, green/orange for the
    /// GM2-without-GM1 text variant.
    fn paint_legacy_scanline(&mut self, row: usize) {
        let ff22 = self.bus.pia1.b.output;
        let border =
            self.bus.gime.color(video::legacy_border_value(ff22));
        let row_px = &mut self.framebuffer
            [row * raster::CANVAS_W * BYTES_PER_PIXEL..][..raster::CANVAS_W * BYTES_PER_PIXEL];

        // Vertical placement from the live LPF bits — the GIME applies LPF
        // even in legacy modes (MAME `update_geometry`).
        let lpf =
            ((self.bus.gime.vres & gime::vres::LPF_MASK) >> gime::vres::LPF_SHIFT) as usize;
        let (top, body) = raster::vertical_window(lpf);
        if row < top || row >= top + body {
            for px in row_px.chunks_exact_mut(BYTES_PER_PIXEL) {
                px.copy_from_slice(&border);
            }
            return;
        }

        // Side borders around the 512 px active span (legacy is always
        // non-wide: MAME `render_scanline`'s `wide = !legacy && ...`).
        for px in row_px[..raster::NON_WIDE_BORDER_X * BYTES_PER_PIXEL]
            .chunks_exact_mut(BYTES_PER_PIXEL)
        {
            px.copy_from_slice(&border);
        }
        for px in row_px[(raster::NON_WIDE_BORDER_X + raster::NON_WIDE_ACTIVE_W)
            * BYTES_PER_PIXEL..]
            .chunks_exact_mut(BYTES_PER_PIXEL)
        {
            px.copy_from_slice(&border);
        }

        // Live per-line mode decode: bytes to fetch and this mode's LPR.
        let ag = ff22 & video::VDG_AG != 0;
        let css = ff22 & video::VDG_CSS != 0;
        let sam_video = self.bus.gime.sam_video;
        let (row_bytes, lines_per_row) = if ag {
            let mode = video::decode_vdg_graphics(ff22, sam_video);
            let lpr = video::LEGACY_GFX_LINES_PER_ROW[(sam_video & 0x07) as usize];
            (mode.bytes_per_row, lpr)
        } else {
            (video::COLS, video::CELL_H)
        };

        // Fetch the current data row through the bus (MMU-honouring logical
        // reads, 16-bit wrap — the legacy renderers' existing data path).
        let (base, line_in_row) = {
            let scan = self.field_scan.as_ref().expect("legacy field latched");
            (scan.row_base as u16, scan.line_in_row)
        };
        let mut buf = [0u8; video::COLS];
        for (i, byte) in buf.iter_mut().take(row_bytes).enumerate() {
            *byte = self.bus.read(base.wrapping_add(i as u16));
        }

        let palette = self.legacy_palette(css);
        let active = &mut self.framebuffer[(row * raster::CANVAS_W
            + raster::NON_WIDE_BORDER_X)
            * BYTES_PER_PIXEL..][..raster::NON_WIDE_ACTIVE_W * BYTES_PER_PIXEL];
        if ag {
            let mode = video::decode_vdg_graphics(ff22, sam_video);
            let indices = video::vdg_palette_indices(mode.bpp, usize::from(css));
            let mut colors = [[0u8; 4]; video::MAX_VDG_COLORS];
            for (slot, &reg) in colors.iter_mut().zip(indices) {
                *slot = palette[reg];
            }
            let xscale = raster::NON_WIDE_ACTIVE_W / mode.logical_w;
            video::paint_legacy_graphics_line(
                &buf[..row_bytes],
                &mode,
                &colors[..indices.len()],
                xscale,
                active,
            );
        } else {
            let generator = video::AlphaGenerator::Gime;
            let xscale = raster::NON_WIDE_ACTIVE_W / (video::COLS * video::CELL_W);
            video::paint_legacy_text_line(
                &buf[..row_bytes],
                &palette,
                generator,
                ff22,
                line_in_row,
                xscale,
                active,
            );
        }

        // Advance the shared vertical counter (MAME `record_full_body_scanline`).
        let scan = self.field_scan.as_mut().expect("legacy field latched");
        scan.line_in_row += 1;
        if scan.line_in_row >= lines_per_row {
            scan.line_in_row = 0;
            scan.row_base += row_bytes;
        }
    }

    /// Current scanline within the field (`0..lines_per_field`): the canonical
    /// raster row being painted (rows ≥ 240 are vertical blanking). Exposed
    /// for scanline-timed tests and debug UI.
    pub fn scanline(&self) -> u32 {
        self.line
    }

    /// Render one video field into `framebuffer` at field end. Only the CoCo
    /// 1/2 renders here — a whole-frame snapshot at the fixed VDG geometry
    /// (those machines have their own raster; the 640×240 canvas is a CoCo 3
    /// GIME artefact). Every CoCo 3 field — GIME-native or legacy — was
    /// already painted line by line ([`Machine::render_scanline`]) and is
    /// complete by the time the field wraps.
    fn render_field(&mut self) {
        if self.config.variant == MachineVariant::Coco3 {
            return;
        }
        if self.bus.pia1.b.output & video::VDG_AG != 0 {
            self.render_coco_graphics();
        } else {
            self.render_coco_text();
        }
    }

    /// Render the legacy CoCo-compatible 32×16 text screen (`DESIGN.md` §6).
    fn render_coco_text(&mut self) {
        self.reset_legacy_fb();
        // Snapshot the text screen through the bus (honours the MMU on CoCo 3;
        // the SAM decode directly on CoCo 1/2) from the display-base register,
        // then render.
        // TODO: per-scanline scanout straight from RAM (`DESIGN.md` §2b/§6).
        let base = self.legacy_display_base();
        let mut screen = [0u8; video::SCREEN_LEN];
        for (i, cell) in screen.iter_mut().enumerate() {
            *cell = self.bus.read(base.wrapping_add(i as u16));
        }
        let ff22 = self.bus.pia1.b.output;
        let css = ff22 & video::VDG_CSS != 0;
        let palette = self.legacy_palette(css);
        // The legacy CoCo-compatible text border is fixed black on both
        // variants (GIME `update_border` / MAME `mc6847.cpp` `border_value`).
        let border = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.color(TEXT_BORDER_COLOR),
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::VDG_FIXED_PALETTE[video::TEXT_BORDER_INDEX]
            }
        };
        // A CoCo 3 has no VDG chip at all: CoCo-compatible text mode is the
        // GIME's own compat-text generator (`video::AlphaGenerator::Gime`),
        // not `self.config.vdg` (which only describes a real CoCo 1/2's VDG
        // and is forced to `Mc6847` for CoCo 3 by `MachineConfig::validate`).
        let generator = match self.config.variant {
            MachineVariant::Coco3 => video::AlphaGenerator::Gime,
            MachineVariant::Coco1 | MachineVariant::Coco2 => match self.config.vdg {
                VDGVariant::MC6847 => video::AlphaGenerator::Mc6847,
                VDGVariant::MC6847T1 => video::AlphaGenerator::Mc6847T1,
            },
        };
        video::render_text(&screen, &palette, border, generator, ff22, &mut self.framebuffer);
    }

    /// Render a VDG bitmap graphics (PMODE) field (`DESIGN.md` §6).
    ///
    /// The mode/colour set come from PIA1 $FF22 and the display base from the
    /// display-base register. Video RAM is read through the bus (honours the
    /// MMU on CoCo 3) from that base — the same low-64K simplification as
    /// `render_coco_text`. The vertical cadence (RAM rows fetched) comes from
    /// the SAM V0-V2 bits: the GIME's own SAM-compat overlay on CoCo 3, the
    /// primary `Sam` on CoCo 1/2 (`docs/coco12-plan.md` Phase 3).
    fn render_coco_graphics(&mut self) {
        self.reset_legacy_fb();
        let ff22 = self.bus.pia1.b.output;
        let sam_video = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.sam_video,
            MachineVariant::Coco1 | MachineVariant::Coco2 => self.bus.sam.v_bits(),
        };
        let mode = video::decode_vdg_graphics(ff22, sam_video);
        let css_bit = ff22 & video::VDG_CSS != 0;
        let css = usize::from(css_bit);
        let indices = video::vdg_palette_indices(mode.bpp, css);
        let palette = self.legacy_palette(css_bit);
        let mut colors = [[0u8; 4]; video::MAX_VDG_COLORS];
        for (slot, &reg) in colors.iter_mut().zip(indices) {
            *slot = palette[reg];
        }

        let base = self.legacy_display_base();
        self.graphics_scratch
            .resize(mode.bytes_per_row * mode.rows, 0);
        for (i, byte) in self.graphics_scratch.iter_mut().enumerate() {
            *byte = self.bus.read(base.wrapping_add(i as u16));
        }

        // The legacy graphics border is not black: MAME `mc6847.cpp`
        // `border_value` returns green (CSS=0) or buff (CSS=1) for graphics
        // modes. CoCo 3 keeps its pre-Phase-3 (black) behaviour unchanged —
        // this fixed-VDG border only applies on the CoCo 1/2 path.
        let border = match self.config.variant {
            MachineVariant::Coco3 => self.bus.gime.color(TEXT_BORDER_COLOR),
            MachineVariant::Coco1 | MachineVariant::Coco2 => {
                video::VDG_FIXED_PALETTE[video::vdg_graphics_border_index(css_bit)]
            }
        };
        let colors = &colors[..indices.len()];
        video::render_graphics(
            &self.graphics_scratch,
            &mode,
            colors,
            border,
            &mut self.framebuffer,
        );
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
