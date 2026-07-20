//! `coco-egui` — eframe frontend. See `DESIGN.md` §8.
//!
//! Boots the real Super Extended Color BASIC ROM, shows the GIME/VDG output as an
//! integer-scaled texture, and feeds host keyboard input into the CoCo matrix in
//! one of two modes (toggle with F12):
//!
//! - Positional — physical key → CoCo matrix position (CoCo applies its own shift
//!   semantics, like MAME). The default.
//! - Symbolic — the character you type is injected via the CoCo keys that produce it.
//!
//! The Machine menu can also insert/eject a cartridge ROM pak (`.rom`/`.ccc`/`.bin`);
//! the debugger panels are still TODO.

mod about;
mod audio;
mod joy;
mod kbd_help;
mod machine_def;
mod manager;
mod new_vm;
mod paper_export;
mod paths;
mod paper_render;
mod paper_view;
mod photo_view;

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, ValueEnum};
use coco_core::cart::{MultiPak, RomPak};
use coco_core::fdc::{DiskCart, JvcDisk};
use coco_core::keyboard::{self as kbd, Pos};
use coco_core::rtc::{DistoRtc, RTCTime};
use coco_core::vhd::VhdImage;
use coco_core::{
    Machine, MachineConfig, MachineVariant, MemorySize, MonitorType, VDGVariant, VideoStandard,
};
use eframe::egui;
use joy::JoystickInputs;
use owo_colors::{OwoColorize, Stream};
use owo_colors::colors::xterm;

/// Integer scale factor for the (small) CoCo framebuffer.
const SCALE: f32 = 3.0;
/// Physical aspect the CoCo frame fills on an NTSC set (4:3). The framebuffer is
/// 288×224 (≈1.29:1); when aspect correction is on, the image is stretched
/// horizontally to this ratio so pixels are ~3% wider than tall, as on real hardware.
const TARGET_ASPECT: f32 = 4.0 / 3.0;
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
const MAX_FRAME_DT: f64 = 0.25;
/// Height reserved for the top menu bar row when sizing the window.
const MENU_BAR_H: f32 = 22.0;
/// Height reserved for the toolbar row when sizing the window.
const TOOLBAR_H: f32 = 30.0;
/// Height reserved for the bottom status bar row when sizing the window.
const STATUS_BAR_H: f32 = 22.0;
/// Symbolic-mode key timing, in fields: hold a synthesized key then release.
const TYPE_HOLD_FIELDS: u8 = 2;
const TYPE_GAP_FIELDS: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum KbMode {
    Positional,
    Symbolic,
}

impl KbMode {
    fn label(self) -> &'static str {
        match self {
            KbMode::Positional => "Positional",
            KbMode::Symbolic => "Symbolic",
        }
    }
}

/// Symbolic-mode type-ahead: replays queued (key, shift) taps with hold/gap timing
/// so the ROM's 60 Hz keyboard scan registers each one.
#[derive(Default)]
struct TypeAhead {
    queue: VecDeque<(Pos, bool)>,
    phase: TypePhase,
    current: (Pos, bool),
}

#[derive(Default, Clone, Copy)]
enum TypePhase {
    #[default]
    Idle,
    Hold(u8),
    Gap(u8),
}

impl TypeAhead {
    fn clear(&mut self) {
        self.queue.clear();
        self.phase = TypePhase::Idle;
    }

    /// True while taps are still queued or a tap is mid hold/gap — i.e. a paste or
    /// type-ahead burst is still draining and owns the keyboard matrix.
    fn is_active(&self) -> bool {
        !self.queue.is_empty() || !matches!(self.phase, TypePhase::Idle)
    }

    /// Advance one field, driving the CoCo matrix for the current tap.
    fn advance(&mut self, kb: &mut kbd::Keyboard) {
        match self.phase {
            TypePhase::Idle => {
                if let Some(entry) = self.queue.pop_front() {
                    self.current = entry;
                    kb.set(entry.0, true);
                    if entry.1 {
                        kb.set(kbd::SHIFT, true);
                    }
                    self.phase = TypePhase::Hold(TYPE_HOLD_FIELDS);
                }
            }
            TypePhase::Hold(0) => {
                kb.set(self.current.0, false);
                kb.set(kbd::SHIFT, false);
                self.phase = TypePhase::Gap(TYPE_GAP_FIELDS);
            }
            TypePhase::Hold(n) => self.phase = TypePhase::Hold(n - 1),
            TypePhase::Gap(0) => self.phase = TypePhase::Idle,
            TypePhase::Gap(n) => self.phase = TypePhase::Gap(n - 1),
        }
    }
}

struct CocoApp {
    machine: Machine,
    texture: Option<egui::TextureHandle>,
    running: bool,
    kb_mode: KbMode,
    type_ahead: TypeAhead,
    show_kbd_help: bool,
    show_about: bool,
    aspect_correct: bool,
    /// Wall-clock instant of the previous update while running; `None` right
    /// after a pause/start so the first frame credits no elapsed time.
    last_update: Option<std::time::Instant>,
    /// Fractional emulated fields owed to the wall clock (`DESIGN.md` §4):
    /// fields run when it reaches 1, the remainder carries over. This decouples
    /// emulation speed from the host refresh rate (120 Hz displays no longer
    /// run the CoCo at double speed).
    field_debt: f64,
    /// Per-port joystick source selection (mouse/gamepad/keys) and gamepad state.
    joysticks: JoystickInputs,
    /// cpal output stream, resampler, and volume/mute state (`audio.rs`).
    audio: audio::AudioOutput,
    /// Letterboxed display rect from the last frame's `CentralPanel`, used to map
    /// pointer position to joystick axes. One frame stale (see `drive_joysticks`).
    display_rect: egui::Rect,
    /// Whether the next inserted cartridge should tie CART* to Q (auto-run at
    /// power-up). Consulted at insert time, not retroactively — see
    /// `RomPak::from_bytes`. Off suits Disk-BASIC-style paks and carts that
    /// must be started with `EXEC &HE010`.
    autostart_cart: bool,
    /// Path of the currently inserted cartridge, if any (shown in the status
    /// bar; also gates the "Eject Cartridge" menu item).
    cart_path: Option<PathBuf>,
    /// Message from the last failed cartridge load, shown in a dismissible
    /// window until acknowledged.
    cart_error: Option<String>,
    /// Source paths of the floppies mounted in the FD-502's drives the UI
    /// exposes (status bar, eject menu items, and write-back targets — a
    /// modified image is written back to its file on eject/replace/exit).
    disk_paths: [Option<PathBuf>; UI_DRIVES],
    /// Source paths of the VHD (virtual hard disk) images mounted in the two
    /// drives the UI exposes (status bar, eject menu items). Unlike
    /// `disk_paths`, VHD writes hit the backing file directly — there is no
    /// in-memory dirty state and so nothing to write back on eject/exit.
    vhd_paths: [Option<PathBuf>; UI_DRIVES],
    /// Source path of the mounted cassette tape (.cas), if any — the
    /// write-back target for recordings, like `disk_paths` for floppies.
    tape_path: Option<PathBuf>,
    /// Whether [`Self::write_back_tape`] should, in addition to the always-
    /// written canonical `.cas`, also synthesize and write a `.wav` of the
    /// tape audio (`coco_core::cassette_wav::synthesize_wav`) alongside it.
    save_tape_wav: bool,
    /// Destination path of the active bit-banger "print to text file"
    /// capture, if any (`docs/printer-plan.md` T2) — shown in the Machine
    /// menu and gates "Stop Print Capture", like `tape_path` does for the
    /// cassette deck. Unlike disk/tape images, there is nothing to write
    /// back on eject: `coco_core::bitbanger::FileSink` writes straight
    /// through as bytes are decoded.
    print_capture_path: Option<PathBuf>,
    /// Machine-menu "Translate CR to LF" checkbox: when set, print captures
    /// rewrite the CoCo's bare-CR line endings as LF so the file reads as
    /// normal host text (faithful raw bytes otherwise). Applies when a
    /// capture starts — an in-progress capture keeps the mode it began with.
    print_capture_lf: bool,
    /// A disk action waiting on the "this will power-cycle the machine"
    /// confirmation dialog — set instead of acting when the FD-502 isn't in
    /// the cartridge slot yet, since inserting it swaps the cartridge and
    /// cold-restarts the machine (unsaved state is lost).
    pending_disk_action: Option<PendingDiskAction>,
    /// State of the inserted Multi-Pak Interface, if any — `None` means the
    /// cartridge slot holds a plain cartridge (or nothing), today's default.
    mpi: Option<MPIState>,
    /// The "Machine → New…" dialog ([`new_vm::NewVmDialog`]): edits a draft
    /// [`MachineConfig`] that [`Self::create_vm`] builds a fresh machine from.
    new_vm: new_vm::NewVmDialog,
    /// True while a Disto RTC is plugged directly into the cartridge port
    /// (gates the "Eject Disto RTC" menu item, like `cart_path` does for ROM
    /// paks). An RTC in a Multi-Pak slot is tracked by [`MPISlot::DistoRTC`]
    /// instead.
    rtc_direct: bool,
    /// The virtual fanfold-paper window (`docs/printer-plan.md` T5), showing
    /// the DMP-105's dot-matrix output on period-correct tractor-feed
    /// stationery. See [`Self::toggle_paper_window`] for the sink-ownership
    /// handshake with print-file-capture.
    paper_window: paper_view::PaperWindow,
}

/// See [`CocoApp::pending_disk_action`].
enum PendingDiskAction {
    Insert { drive: usize, path: PathBuf },
    NewBlank { drive: usize, path: PathBuf },
}

/// A window title styled uniformly across the app: sized to the button font
/// and strong (bold). Applied to every [`egui::Window`] title so they match.
/// We can't just resize `TextStyle::Heading` globally (egui's window-title
/// fallback) because content `ui.heading()` calls share that style.
pub(crate) fn window_title(ctx: &egui::Context, text: &str) -> egui::RichText {
    let size = ctx.style().text_styles[&egui::TextStyle::Button].size;
    egui::RichText::new(text).size(size).strong()
}

/// Drives the UI exposes. The FD-502 latch can address four, but real setups
/// were one or two — and the menu stays small.
const UI_DRIVES: usize = 2;

/// Status-bar drive activity indicator: a little 5¼" floppy jacket, red
/// while the drive is selected with its motor on ([`coco_core::fdc`]'s
/// `drive_active`, like a real drive's front-panel light), dim otherwise.
const DRIVE_ICON_SIZE: f32 = 11.0;
const DRIVE_ICON_ACTIVE: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x30, 0x30);
const DRIVE_ICON_IDLE: egui::Color32 = egui::Color32::from_gray(70);
/// Corner rounding of the jacket square.
const DRIVE_ICON_CORNER: f32 = 1.5;

/// Status-bar cassette activity indicator, the tape sibling of
/// [`DRIVE_ICON_SIZE`]'s floppy: shell proportions of a compact cassette
/// (wider than tall), red while the cassette relay is closed
/// (CLOAD/CSAVE/`MOTOR ON`), dim otherwise.
const TAPE_ICON_SIZE: egui::Vec2 = egui::vec2(14.0, 10.0);
/// Corner rounding of the cassette shell.
const TAPE_ICON_CORNER: f32 = 1.5;

/// One status-bar cassette indicator (see [`TAPE_ICON_SIZE`]'s doc): the
/// shell with the two reel hubs punched out in the panel's background
/// color.
fn cassette_activity_light(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(TAPE_ICON_SIZE, egui::Sense::hover());
    let shell = if active { DRIVE_ICON_ACTIVE } else { DRIVE_ICON_IDLE };
    let punch = ui.visuals().panel_fill;
    let painter = ui.painter();
    painter.rect_filled(rect, TAPE_ICON_CORNER, shell);
    // The two reel hubs, side by side above the mid-line (the head window
    // occupies a real shell's bottom edge, unreadable at this size).
    let hub_y = rect.center().y - TAPE_ICON_SIZE.y * 0.08;
    let hub_dx = TAPE_ICON_SIZE.x * 0.22;
    let hub_r = TAPE_ICON_SIZE.y * 0.20;
    painter.circle_filled(egui::pos2(rect.center().x - hub_dx, hub_y), hub_r, punch);
    painter.circle_filled(egui::pos2(rect.center().x + hub_dx, hub_y), hub_r, punch);
}

/// One status-bar activity indicator (see [`DRIVE_ICON_SIZE`]'s doc): the
/// jacket square with the hub hole and the oblong head-access slot punched
/// out in the panel's background color — the 5¼" silhouette.
fn drive_activity_light(ui: &mut egui::Ui, active: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(DRIVE_ICON_SIZE, DRIVE_ICON_SIZE),
        egui::Sense::hover(),
    );
    let jacket = if active { DRIVE_ICON_ACTIVE } else { DRIVE_ICON_IDLE };
    let punch = ui.visuals().panel_fill;
    let painter = ui.painter();
    painter.rect_filled(rect, DRIVE_ICON_CORNER, jacket);
    // Hub hole, a hair above center (the slot below claims the bottom).
    let hub = rect.center() - egui::vec2(0.0, DRIVE_ICON_SIZE * 0.08);
    painter.circle_filled(hub, DRIVE_ICON_SIZE * 0.18, punch);
    // Head-access slot: the short oblong under the hub.
    let slot_width = DRIVE_ICON_SIZE * 0.16;
    let slot = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - DRIVE_ICON_SIZE * 0.18),
        egui::vec2(slot_width, DRIVE_ICON_SIZE * 0.24),
    );
    painter.rect_filled(slot, slot_width / 2.0, punch);
}

/// Number of physical slots on a Multi-Pak Interface — re-exported from the
/// core crate's own constant so the frontend's slot arrays can't drift from
/// [`coco_core::cart::MultiPak`]'s.
const MPI_SLOT_COUNT: usize = coco_core::cart::mpi::SLOT_COUNT;

/// Front-panel MPI switch position an [`MultiPak`] starts on
/// ([`CocoApp::insert_multipak`]): slot 4, the conventional disk-controller
/// default (also MAME's default — `coco_multi.cpp` `MULTI_SLOT_LOOKUP`).
const DEFAULT_MPI_SWITCH_SLOT: usize = MPI_SLOT_COUNT - 1;

/// MPI slot `--rtc` targets (slot 3): --cart takes slot 1 and the FD-502
/// slot 4, mirroring the conventional layout the `--mpi` CLI wiring builds.
const DEFAULT_RTC_SLOT: usize = 2;

/// What occupies one Multi-Pak Interface slot, tracked by the frontend so a
/// cold restart (or just the status bar / menu labels) can describe it
/// without having to downcast the core's trait objects. The FD-502 doesn't
/// carry its own disk paths here — those stay in [`CocoApp::disk_paths`]
/// exactly as they do without an MPI, since [`Cartridge::as_disk_cart`]
/// forwarding already makes the drive UI transparent to whether the
/// controller lives at the top level or nested in a slot.
///
/// [`Cartridge::as_disk_cart`]: coco_core::cart::Cartridge::as_disk_cart
enum MPISlot {
    Empty,
    ROMPak(PathBuf),
    FD502,
    DistoRTC,
}

/// Frontend-tracked state of an inserted [`MultiPak`]: which slot the
/// front-panel switch points at (mirrors [`MultiPak::set_switch`]) and what's
/// plugged into each of its 4 slots ([`MPISlot`]).
struct MPIState {
    switch: usize,
    slots: [MPISlot; MPI_SLOT_COUNT],
}

/// The host's local wall clock, read once (RTC sync).
fn host_now() -> RTCTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    RTCTime {
        year: now.year(),
        month: now.month() as u8,
        day: now.day() as u8,
        hour: now.hour() as u8,
        minute: now.minute() as u8,
        second: now.second() as u8,
    }
}

/// [`host_now`] as the Disto RTC's injected time source
/// (`coco_core::rtc::TimeSource` — coco-core itself never reads `std::time`).
fn host_time_source() -> coco_core::rtc::TimeSource {
    Box::new(host_now)
}

impl CocoApp {
    /// `CreationContext` isn't taken here (unlike most `eframe::App`
    /// constructors): nothing in this struct's setup touches egui context
    /// state (fonts, wgpu/glow handles), so it's a plain constructor
    /// callable from anywhere a machine needs to be built — the direct-boot
    /// `main()` (which does have a `CreationContext` in its `run_native`
    /// closure but never needed to pass it in) and the CocoVM manager's
    /// `launch_machine` (`plan-machine-persistence.md` step 5), which builds
    /// VMs from inside `ManagerApp::update` where no `CreationContext`
    /// exists at all.
    fn new(
        config: MachineConfig,
        rom: Box<[u8]>,
        cart_path: Option<PathBuf>,
        disk_paths: [Option<PathBuf>; UI_DRIVES],
        vhd_paths: [Option<PathBuf>; UI_DRIVES],
        save_tape_wav: bool,
    ) -> Self {
        let mut app = Self {
            machine: Machine::new(config, rom),
            texture: None,
            running: true, // boot straight to the prompt
            kb_mode: KbMode::Positional,
            type_ahead: TypeAhead::default(),
            show_kbd_help: false,
            show_about: false,
            aspect_correct: true,
            last_update: None,
            field_debt: 0.0,
            joysticks: JoystickInputs::new(),
            display_rect: egui::Rect::NOTHING,
            audio: audio::AudioOutput::new(),
            autostart_cart: true,
            cart_path: None,
            cart_error: None,
            disk_paths: [None, None],
            vhd_paths: [None, None],
            tape_path: None,
            save_tape_wav,
            print_capture_path: None,
            print_capture_lf: false,
            pending_disk_action: None,
            mpi: None,
            new_vm: new_vm::NewVmDialog::new(),
            rtc_direct: false,
            paper_window: paper_view::PaperWindow::new(),
        };
        if let Some(path) = cart_path {
            app.insert_cartridge(path);
        }
        for (drive, path) in disk_paths.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_disk(drive, path);
            }
        }
        for (drive, path) in vhd_paths.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_vhd(drive, path);
            }
        }
        app
    }

    /// Load a ROM pak from `path` and insert it, using the current
    /// `autostart_cart` setting. Resets the machine on success (cartridge
    /// insertion is a machine-off operation on real hardware); on failure,
    /// leaves the running cartridge (if any) untouched and records the error
    /// for [`Self::cart_error`] to display.
    fn insert_cartridge(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match RomPak::from_bytes(&bytes, self.autostart_cart) {
            Ok(pak) => {
                self.flush_dirty_disks();
                self.machine.insert_cartridge(Box::new(pak));
                self.machine.power_cycle();
                self.cart_path = Some(path);
                self.disk_paths = [None, None];
                self.mpi = None; // plugging straight into the port removes any MPI
                self.rtc_direct = false; // ... and any directly-plugged RTC
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Eject the current cartridge and power-cycle the machine (cartridge
    /// swaps are machine-off operations on real hardware).
    fn eject_cartridge(&mut self) {
        self.flush_dirty_disks();
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None; // whatever was plugged into the port (MPI or not) is gone
        self.rtc_direct = false;
    }

    /// Make sure the inserted cartridge is the FD-502 disk controller,
    /// creating one (with `roms/disk11.rom`) if something else — or nothing —
    /// is in the slot. Creating it cold-resets the machine: BASIC only probes
    /// for Disk BASIC at cold start. Swapping a floppy in an already-present
    /// controller does NOT reset, like on real hardware.
    ///
    /// With a Multi-Pak Interface installed, the slot to plug the FD-502 into
    /// is a real choice a top-level "just ensure a controller exists" call
    /// can't make on the caller's behalf — so this refuses instead of
    /// silently replacing the MPI, and directs the caller to
    /// [`Self::mpi_insert_fd502`] via the MultiPak submenu.
    fn ensure_disk_controller(&mut self) -> Result<(), String> {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            return Ok(());
        }
        if self.mpi.is_some() {
            return Err(
                "No FD-502 is installed in the MultiPak. Use Machine > MultiPak Interface > \
                 a slot > Insert FD-502 first."
                    .to_string(),
            );
        }
        // TODO! will need to read from config ~/.share/cocovm or something, there should be some helper for this, maybe in paths.rs
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/disk11.rom");
        let rom = std::fs::read(&path)
            .map_err(|e| format!("could not read Disk BASIC ROM {}: {e}", path.display()))?;
        report_rom_validation(&path, &rom);
        self.flush_dirty_disks();
        self.machine.insert_cartridge(Box::new(DiskCart::new(rom.into_boxed_slice())));
        // Power cycle, not warm reset: the DK probe that links Disk BASIC
        // only runs on the ROM's cold-start path (a warm reset leaves the
        // DOS ROM unlinked and the drives dead).
        self.machine.power_cycle();
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.rtc_direct = false;
        Ok(())
    }

    /// Insert a Multi-Pak Interface into the cartridge slot (cold-restart
    /// gated, like plain cartridge insertion): swaps out whatever was
    /// plugged directly into the port for an empty 4-slot MPI with its
    /// front-panel switch on slot 4 ([`DEFAULT_MPI_SWITCH_SLOT`]).
    fn insert_multipak(&mut self) {
        self.flush_dirty_disks();
        self.machine.insert_cartridge(Box::new(MultiPak::new(DEFAULT_MPI_SWITCH_SLOT)));
        self.machine.power_cycle();
        self.mpi = Some(MPIState {
            switch: DEFAULT_MPI_SWITCH_SLOT,
            slots: std::array::from_fn(|_| MPISlot::Empty),
        });
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.rtc_direct = false;
    }

    /// Remove the Multi-Pak Interface — and everything plugged into it —
    /// restoring the plain empty cartridge slot.
    fn eject_multipak(&mut self) {
        self.flush_dirty_disks();
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.mpi = None;
        self.cart_path = None;
        self.disk_paths = [None, None];
    }

    /// Load a ROM pak into MPI `slot` (0-3), using the current
    /// `autostart_cart` setting. Mirrors [`Self::insert_cartridge`] but
    /// targets one slot of the already-inserted MPI instead of the whole
    /// cartridge port.
    fn mpi_insert_rompak(&mut self, slot: usize, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match RomPak::from_bytes(&bytes, self.autostart_cart) {
            Ok(pak) => {
                self.flush_dirty_disks();
                if let Some(mp) = self.machine.bus.cart.as_multipak() {
                    mp.insert(slot, Box::new(pak));
                }
                if let Some(mpi) = &mut self.mpi {
                    mpi.slots[slot] = MPISlot::ROMPak(path);
                }
                self.machine.power_cycle();
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Insert the FD-502 disk controller into MPI `slot`, unless one is
    /// already installed in a different slot (the FD-502 latch only ever
    /// models one controller). Mirrors [`Self::ensure_disk_controller`]'s
    /// cold-start rationale, but targets one MPI slot instead of the whole
    /// cartridge port.
    fn mpi_insert_fd502(&mut self, slot: usize) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.cart_error = Some("An FD-502 is already installed in another slot.".to_string());
            return;
        }
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/disk11.rom");
        let rom = match std::fs::read(&path) {
            Ok(rom) => rom,
            Err(e) => {
                self.cart_error =
                    Some(format!("could not read Disk BASIC ROM {}: {e}", path.display()));
                return;
            }
        };
        self.flush_dirty_disks();
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, Box::new(DiskCart::new(rom.into_boxed_slice())));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::FD502;
        }
        self.disk_paths = [None, None];
        self.machine.power_cycle();
    }

    /// Eject whatever is plugged into MPI `slot`, restoring its empty slot.
    fn mpi_eject_slot(&mut self, slot: usize) {
        let was_fd502 = matches!(self.mpi.as_ref().map(|m| &m.slots[slot]), Some(MPISlot::FD502));
        if was_fd502 {
            self.flush_dirty_disks();
            self.disk_paths = [None, None];
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.eject(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::Empty;
        }
        self.machine.power_cycle();
    }

    /// Move the MPI's front-panel switch to `slot`. A running program's own
    /// write to `$FF7F` overrides the switch until the next reset
    /// ([`coco_core::cart::MultiPak::set_switch`]).
    fn mpi_set_switch(&mut self, slot: usize) {
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.set_switch(slot);
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.switch = slot;
        }
    }

    /// Plug a Disto RTC directly into the cartridge port, running on the
    /// host's local clock (cold-restart gated like any cartridge swap). The
    /// RTC has no boot ROM, so this pairs with a VHD boot (NitrOS-9 `emudsk`)
    /// rather than the FD-502 — for RTC + floppies, use a Multi-Pak slot.
    fn insert_rtc(&mut self) {
        self.flush_dirty_disks();
        self.machine.insert_cartridge(Box::new(DistoRtc::new(host_time_source())));
        self.machine.power_cycle();
        self.rtc_direct = true;
        self.cart_path = None;
        self.disk_paths = [None, None];
        self.mpi = None;
    }

    /// Eject a directly-plugged Disto RTC, restoring the empty port.
    fn eject_rtc(&mut self) {
        self.machine.eject_cartridge();
        self.machine.power_cycle();
        self.rtc_direct = false;
    }

    /// Insert a Disto RTC into MPI `slot` (0-3). Mirrors
    /// [`Self::mpi_insert_fd502`]; only one RTC is allowed across the
    /// machine, since two would shadow each other at `$FF50`.
    fn mpi_insert_rtc(&mut self, slot: usize) {
        if self.machine.bus.cart.as_disto_rtc().is_some() {
            self.cart_error = Some("A Disto RTC is already installed in another slot.".to_string());
            return;
        }
        if let Some(mp) = self.machine.bus.cart.as_multipak() {
            mp.insert(slot, Box::new(DistoRtc::new(host_time_source())));
        }
        if let Some(mpi) = &mut self.mpi {
            mpi.slots[slot] = MPISlot::DistoRTC;
        }
        self.machine.power_cycle();
    }

    /// Set the emulated RTC (wherever it is — port or MPI slot) back to the
    /// host's clock, discarding any offset a guest-side `setime` introduced.
    fn sync_rtc_to_host(&mut self) {
        if let Some(rtc) = self.machine.bus.cart.as_disto_rtc() {
            rtc.rtc().set_time(host_now());
        }
    }

    /// Menu-path entry for Insert Disk: acts immediately when the FD-502 is
    /// already in the slot; otherwise parks the action behind the
    /// power-cycle confirmation dialog (see [`Self::pending_disk_action`]).
    fn request_insert_disk(&mut self, drive: usize, path: PathBuf) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.insert_disk(drive, path);
        } else {
            self.pending_disk_action = Some(PendingDiskAction::Insert { drive, path });
        }
    }

    /// Menu-path entry for New Blank Disk, gated like [`Self::request_insert_disk`].
    fn request_new_blank_disk(&mut self, drive: usize, path: PathBuf) {
        if self.machine.bus.cart.as_disk_cart().is_some() {
            self.new_blank_disk(drive, path);
        } else {
            self.pending_disk_action = Some(PendingDiskAction::NewBlank { drive, path });
        }
    }

    /// Mount the floppy image at `path` in `drive`, inserting the FD-502
    /// controller first if needed. Failures land in [`Self::cart_error`].
    fn insert_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
            let bytes =
                std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
            let disk =
                JvcDisk::from_bytes(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            self.write_back_disk(drive); // whatever was in the drive first
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Create a brand-new, blank (0-track) floppy image at `path` and mount it
    /// in `drive`, inserting the FD-502 controller first if needed. Refuses to
    /// overwrite an existing file. Failures land in [`Self::cart_error`].
    fn new_blank_disk(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            self.ensure_disk_controller()?;
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!(
                        "{} already exists; use Insert Disk to mount an existing image, or \
                         choose a different name",
                        path.display()
                    ));
                }
                Err(e) => return Err(format!("could not create {}: {e}", path.display())),
            }
            let disk = JvcDisk::from_bytes(Vec::new()).map_err(|e| format!("{}: {e}", path.display()))?;
            self.write_back_disk(drive); // whatever was in the drive first
            let cart = self.machine.bus.cart.as_disk_cart().expect("just ensured");
            cart.insert_disk(drive, disk);
            self.disk_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Eject the floppy in `drive`, writing a modified image back to its file
    /// first (like MAME/VCC, in-place).
    fn eject_disk(&mut self, drive: usize) {
        self.write_back_disk(drive);
        if let Some(cart) = self.machine.bus.cart.as_disk_cart() {
            cart.eject_disk(drive);
        }
        self.disk_paths[drive] = None;
    }

    /// If the floppy in `drive` was written to, save the image back to its
    /// source file. Failures land in [`Self::cart_error`] (the in-memory disk
    /// is left mounted and still dirty, so a later retry can succeed).
    fn write_back_disk(&mut self, drive: usize) {
        let Some(path) = self.disk_paths[drive].clone() else {
            return;
        };
        let Some(cart) = self.machine.bus.cart.as_disk_cart() else {
            return;
        };
        let Some(disk) = cart.disk(drive) else {
            return;
        };
        if !disk.dirty() {
            return;
        }
        if let Err(e) = std::fs::write(&path, disk.bytes()) {
            self.cart_error = Some(format!("could not save {}: {e}", path.display()));
        }
    }

    /// Write every modified floppy back to its file (controller swap, exit).
    fn flush_dirty_disks(&mut self) {
        for drive in 0..UI_DRIVES {
            self.write_back_disk(drive);
        }
    }

    /// Mount the VHD image at `path` in `drive`. Unlike floppies, VHD is a
    /// bus-level device (`$FF80-$FF86`, `SystemBus::vhd`) independent of the
    /// cartridge slot: no controller to ensure, no machine reset, and no
    /// write-back on eject/replace (VHD command execution writes straight
    /// through to the backing file). Failures land in [`Self::cart_error`].
    fn insert_vhd(&mut self, drive: usize, path: PathBuf) {
        let result = (|| -> Result<(), String> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| format!("could not open {}: {e}", path.display()))?;
            self.machine.bus.vhd.insert(drive, VhdImage::File(file));
            self.vhd_paths[drive] = Some(path);
            Ok(())
        })();
        if let Err(e) = result {
            self.cart_error = Some(e);
        }
    }

    /// Eject the VHD image in `drive`. No write-back: VHD writes already hit
    /// the backing file directly.
    fn eject_vhd(&mut self, drive: usize) {
        self.machine.bus.vhd.eject(drive);
        self.vhd_paths[drive] = None;
    }

    /// Mount the tape at `path` (.cas decoded bytes, or a .wav recording
    /// demodulated via [`coco_core::cassette_wav::decode_wav`] — sniffed by
    /// the `RIFF` magic on the loaded bytes, not the file extension, since a
    /// picked file's extension isn't authoritative), writing back whatever
    /// was in the deck first. Failures land in [`Self::cart_error`] and
    /// leave the currently mounted tape untouched.
    fn insert_tape(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        let tape = if bytes.starts_with(b"RIFF") {
            match coco_core::cassette_wav::decode_wav(&bytes, self.machine.cpu_hz()) {
                Ok(tape) => tape,
                Err(e) => {
                    self.cart_error = Some(format!("could not decode {}: {e}", path.display()));
                    return;
                }
            }
        } else {
            bytes
        };
        self.write_back_tape();
        self.machine.bus.cassette.insert_tape(tape);
        self.tape_path = Some(path);
    }

    /// Create a brand-new blank tape at `path` and mount it, ready for CSAVE.
    /// Refuses to overwrite an existing file (mirrors [`Self::new_blank_disk`]).
    fn new_tape(&mut self, path: PathBuf) {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {
                self.write_back_tape();
                self.machine.bus.cassette.insert_tape(Vec::new());
                self.tape_path = Some(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                self.cart_error = Some(format!(
                    "{} already exists; use Insert Tape to mount an existing tape, or \
                     choose a different name",
                    path.display()
                ));
            }
            Err(e) => {
                self.cart_error = Some(format!("could not create {}: {e}", path.display()));
            }
        }
    }

    /// Eject the tape, saving an unfinished recording back to its file first.
    fn eject_tape(&mut self) {
        self.write_back_tape();
        self.machine.bus.cassette.eject_tape();
        self.tape_path = None;
    }

    /// Finalize any pending recording and, if the tape changed, save it back
    /// (like [`Self::write_back_disk`]; on failure the tape stays mounted
    /// and dirty so a later retry can succeed).
    ///
    /// The canonical save is always a `.cas` — `tape_path` with its
    /// extension forced to `.cas` (a no-op if it already was one, e.g. a
    /// tape mounted from `.cas` to begin with; `foo.wav` becomes `foo.cas`).
    /// On success, `tape_path` is updated to that `.cas` path so a tape
    /// originally mounted from a `.wav` is never silently overwritten
    /// again — from then on the app tracks the `.cas` sibling. When
    /// [`Self::save_tape_wav`] is on, a `.wav` of the tape audio
    /// ([`coco_core::cassette_wav::synthesize_wav`]) is additionally
    /// written alongside it, next to (not instead of) the `.cas`.
    fn write_back_tape(&mut self) {
        self.machine.bus.cassette.finalize_recording();
        let Some(path) = self.tape_path.clone() else {
            return;
        };
        if !self.machine.bus.cassette.dirty() {
            return;
        }
        let cas_path = path.with_extension("cas");
        match std::fs::write(&cas_path, self.machine.bus.cassette.tape_bytes()) {
            Ok(()) => {
                self.machine.bus.cassette.mark_saved();
                if cas_path != path {
                    self.tape_path = Some(cas_path.clone());
                }
            }
            Err(e) => {
                self.cart_error = Some(format!("could not save {}: {e}", cas_path.display()));
                return;
            }
        }
        if self.save_tape_wav {
            let wav_path = cas_path.with_extension("wav");
            let wav = coco_core::cassette_wav::synthesize_wav(
                self.machine.bus.cassette.tape_bytes(),
                self.machine.cpu_hz(),
            );
            if let Err(e) = std::fs::write(&wav_path, wav) {
                self.cart_error = Some(format!("could not save {}: {e}", wav_path.display()));
            }
        }
    }

    /// Start "print to text file" capture at `path` (create/truncate —
    /// [`coco_core::bitbanger::BitBanger::start_file_capture`]). Failures
    /// (e.g. an unwritable path) land in [`Self::cart_error`] and leave any
    /// previous capture running.
    ///
    /// Symmetric with [`Self::toggle_paper_window`]: if the paper window
    /// currently owns the bit-banger's sink, starting file capture yanks it
    /// out from under the window, so the window is detached (and closed)
    /// rather than left showing stale content.
    fn start_print_capture(&mut self, path: PathBuf) {
        match self
            .machine
            .bus
            .bitbanger
            .start_file_capture(&path, self.print_capture_lf)
        {
            Ok(()) => {
                self.print_capture_path = Some(path);
                self.paper_window.detach();
            }
            Err(e) => self.cart_error = Some(format!("could not open {}: {e}", path.display())),
        }
    }

    /// Stop capture, restoring the bit-banger's no-op sink.
    fn stop_print_capture(&mut self) {
        self.machine.bus.bitbanger.stop_capture();
        self.print_capture_path = None;
    }

    /// View-menu "Printer Paper" checkbox handler: on closed->open,
    /// attaches a DMP-105 to the bit-banger if the paper window doesn't
    /// already have a live handle (stopping any active print-file-capture
    /// first, since only one sink is live at a time). Closing just hides
    /// the window — the handle stays attached so it keeps accumulating
    /// output in the background (see `paper_view`'s module doc comment).
    fn toggle_paper_window(&mut self) {
        if self.paper_window.open {
            self.paper_window.open = false;
            return;
        }
        if self.paper_window.handle.is_none() {
            if self.print_capture_path.is_some() {
                self.stop_print_capture();
            }
            self.paper_window.handle = Some(self.machine.bus.bitbanger.start_dmp105());
        }
        self.paper_window.open = true;
    }

    /// Build a brand-new machine from `config`, replacing the current one
    /// wholesale (the "New…" dialog's Create). The ROM set for the chosen
    /// variant is loaded first, so a failure (returned for the dialog to
    /// display) leaves the running machine untouched. On success, dirty
    /// floppies and tape are written back exactly like [`Self::on_exit`],
    /// then every mounted device and frontend path is dropped — a new VM
    /// starts bare, like a machine fresh out of the box. Sticky UI
    /// preferences (keyboard mode, joysticks, audio, autostart, CR→LF)
    /// survive; they belong to the app, not the machine.
    fn create_vm(&mut self, config: MachineConfig, ctx: &egui::Context) -> Result<(), String> {
        let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
        let rom = load_default_rom(config.variant, &roms_dir)?;

        self.flush_dirty_disks();
        self.write_back_tape();
        // The bit-banger sinks (file capture / DMP-105 paper feed) belong to
        // the old machine and drop with it; clear the frontend's handles so
        // neither UI points at a dead device.
        self.print_capture_path = None;
        self.paper_window.detach();

        self.machine = Machine::new(config, rom);
        // `self.texture` is deliberately left alone: nulling it here would
        // panic in this same frame's CentralPanel (drawn after the dialog),
        // and the per-frame `texture.set` at the top of `update` re-uploads
        // the new machine's framebuffer — including a size change, CoCo 1/2
        // and CoCo 3 framebuffers differ — on the next pass anyway.
        self.running = true; // boot straight to the prompt, like startup
        self.type_ahead.clear();
        self.last_update = None;
        self.field_debt = 0.0;
        self.cart_path = None;
        self.cart_error = None;
        self.disk_paths = [None, None];
        self.vhd_paths = [None, None];
        self.tape_path = None;
        self.pending_disk_action = None;
        self.mpi = None;
        self.rtc_direct = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "coco-rs — {}",
            machine_label(config.variant)
        )));
        Ok(())
    }

    /// Emulated fields owed for this update, from wall-clock time at the
    /// machine's field rate (60 Hz NTSC / 50 Hz PAL).
    fn fields_due(&mut self) -> usize {
        let now = std::time::Instant::now();
        let dt = match self.last_update.replace(now) {
            Some(prev) => (now - prev).as_secs_f64().min(MAX_FRAME_DT),
            None => 0.0,
        };
        self.field_debt += dt * self.machine.config.video.field_rate_hz();
        let due = (self.field_debt as usize).min(MAX_FIELDS_PER_UPDATE);
        self.field_debt = (self.field_debt - due as f64).min(1.0);
        due
    }

    fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.machine.bus.keyboard.release_all();
            self.type_ahead.clear();
        }
    }

    /// Queue a string as symbolic key taps (used by clipboard paste and, in symbolic
    /// mode, typed text). Characters with no CoCo key are skipped; `\n`/`\r` → ENTER.
    fn enqueue_text(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(entry) = kbd::char_key(c) {
                self.type_ahead.queue.push_back(entry);
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        // ⌘N / Ctrl+N = Machine → New…. Consumed before the event snapshot
        // below so the N keypress never reaches the CoCo matrix or the
        // symbolic type-ahead (the held modifier alone is harmless there).
        if ctx.input_mut(|i| i.consume_shortcut(&new_vm::NEW_MACHINE_SHORTCUT)) {
            self.new_vm.open_with(self.machine.config);
        }

        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));

        // UI hotkeys (never forwarded) and clipboard paste, both keyboard-mode-agnostic.
        // egui/eframe normalises the platform paste shortcut (Cmd+V / Ctrl+V) into a
        // single Event::Paste, so this works the same on macOS, Windows, and Linux.
        for ev in &events {
            match ev {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => match key {
                    egui::Key::F12 => {
                        let next = match self.kb_mode {
                            KbMode::Positional => KbMode::Symbolic,
                            KbMode::Symbolic => KbMode::Positional,
                        };
                        self.set_mode(next);
                    }
                    egui::Key::F10 => self.show_kbd_help = !self.show_kbd_help,
                    egui::Key::F9 => self.aspect_correct = !self.aspect_correct,
                    _ => {}
                },
                egui::Event::Paste(text) => self.enqueue_text(text),
                _ => {}
            }
        }

        // Symbolic mode also turns typed characters and control keys into queued taps.
        // Arrows are skipped when a joystick port is in Keys mode (see below).
        if self.kb_mode == KbMode::Symbolic {
            let joystick_keys = self.joysticks.keys_active();
            for ev in &events {
                match ev {
                    egui::Event::Text(text) => self.enqueue_text(text),
                    egui::Event::Key { key, pressed: true, .. } => {
                        if joystick_keys && is_joystick_key(*key) {
                            continue;
                        }
                        if let Some(pos) = control_key_pos(*key) {
                            self.type_ahead.queue.push_back((pos, false));
                        }
                    }
                    _ => {}
                }
            }
        }

        // While a paste / type-ahead burst is draining it owns the matrix, in either
        // mode, so replayed taps aren't clobbered by the per-frame positional writes.
        // (The taps themselves advance once per *emulated field*, in `update`.)
        if self.type_ahead.is_active() {
            return;
        }

        // Positional mode: physical keys drive the CoCo matrix directly. Arrows and
        // Z/X are skipped when a joystick port is in Keys mode, so the two input
        // paths don't fight over the same physical keys.
        if self.kb_mode == KbMode::Positional {
            let joystick_keys = self.joysticks.keys_active();
            let kb = &mut self.machine.bus.keyboard;
            kb.set(kbd::SHIFT, mods.shift);
            kb.set(kbd::CTRL, mods.ctrl);
            kb.set(kbd::ALT, mods.alt);
            for ev in &events {
                if let egui::Event::Key { key, physical_key, pressed, .. } = ev {
                    let k = physical_key.unwrap_or(*key);
                    if k == egui::Key::F12 {
                        continue;
                    }
                    if joystick_keys && is_joystick_key(k) {
                        continue;
                    }
                    if let Some(pos) = key_to_pos(k) {
                        kb.set(pos, *pressed);
                    }
                }
            }
        }
    }

    /// Poll and apply all joystick input sources (mouse/gamepad/keys) for both
    /// ports. Called once per `update()`, before running any emulated fields, so
    /// the pot/button state a field sees is this frame's, not last frame's.
    fn drive_joysticks(&mut self, ctx: &egui::Context) {
        self.joysticks.apply(ctx, self.display_rect, &mut self.machine);
    }

    /// Write modified floppies and tape back to their files — the exit
    /// contract [`eframe::App::on_exit`] runs for the direct-boot window,
    /// and the same one a manager-owned VM needs on Stop or on the
    /// manager's own `on_exit` (`ManagerApp`'s `eframe::App` impl in
    /// `manager.rs`, `docs/plan-machine-persistence.md` "one native window
    /// per running VM").
    fn flush_media(&mut self) {
        self.flush_dirty_disks();
        self.write_back_tape();
    }

    /// Toggle Run/Pause, the same assignment the toolbar and Machine-menu
    /// "Pause"/"Run" buttons perform in [`Self::draw_chrome`] — exposed so
    /// the manager's detail-pane Pause/Resume button can drive a VM it
    /// doesn't otherwise reach into (`running` has no `pub` visibility).
    pub(crate) fn toggle_running(&mut self) {
        self.running = !self.running;
    }

    /// Whether the VM is currently running (vs. paused) — the manager's
    /// list-row and detail-pane status label reads this instead of the
    /// (never-persisted) `Stopped` placeholder every row used to show.
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    /// The framebuffer texture [`Self::step_emulation`] uploads every
    /// frame — `None` only before the VM's very first frame runs. Exposed
    /// so the manager's list-row thumbnail
    /// (`docs/plan-machine-persistence.md` step 6, "Running/paused VM"
    /// bullet) can draw the *same* `TextureHandle` in a second place: one
    /// `egui::Context` serves every viewport, so reusing the handle here
    /// costs one extra quad, not an extra upload — and a paused VM's
    /// texture simply stops changing, so the thumbnail naturally freezes on
    /// its last frame with no special-casing needed.
    pub(crate) fn framebuffer_texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// Advance emulation for one host frame — input, joysticks, the
    /// wall-clock-paced field loop, audio, and the framebuffer texture
    /// upload. Runs regardless of which chrome (if any) is drawn around the
    /// display this frame: [`Self::window_ui`] (full native window) and the
    /// manager's `ViewportClass::Embedded` fallback both call this before
    /// drawing anything, so a VM keeps emulating even in the degraded
    /// single-window case (`docs/plan-machine-persistence.md` "one native
    /// window per running VM").
    fn step_emulation(&mut self, ctx: &egui::Context) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            // Run however many fields the wall clock owes us (real-time pacing),
            // stepping type-ahead per field so paste timing is refresh-agnostic.
            for _ in 0..self.fields_due() {
                if self.type_ahead.is_active() {
                    self.type_ahead.advance(&mut self.machine.bus.keyboard);
                }
                self.machine.run_field();
            }
            let sample_rate = self.machine.audio_sample_rate();
            self.audio.push_samples(self.machine.take_audio(), sample_rate);
            ctx.request_repaint();
        } else {
            self.last_update = None;
        }

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [
                self.machine.fb_width as usize,
                self.machine.fb_height as usize,
            ],
            &self.machine.framebuffer,
        );
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture("coco-fb", image.clone(), egui::TextureOptions::NEAREST)
        });
        texture.set(image, egui::TextureOptions::NEAREST);
    }

    /// The menu bar, toolbar, status bar, and every optional window/dialog
    /// (keyboard help, About, the "New…" dialog, the printer-paper window,
    /// the disk-controller confirmation, the cartridge-error banner) — every
    /// bit of chrome around the CoCo display itself. Split out of
    /// [`Self::window_ui`] so the manager's `ViewportClass::Embedded`
    /// fallback can skip it entirely: drawing two apps' menu bars/status
    /// bars into one shared `ctx` would interleave them into a single
    /// confusing window, so that fallback shows only [`Self::draw_display`]
    /// (`docs/plan-machine-persistence.md` "one native window per running
    /// VM").
    fn draw_chrome(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Machine", |ui| {
                    let new_button = egui::Button::new("New…")
                        .shortcut_text(ui.ctx().format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT));
                    if ui.add(new_button).clicked() {
                        self.new_vm.open_with(self.machine.config);
                        ui.close();
                    }
                    ui.separator();
                    let run_label = if self.running { "Pause" } else { "Run" };
                    if ui.button(run_label).clicked() {
                        self.running = !self.running;
                        ui.close();
                    }
                    if ui.button("Reset").clicked() {
                        self.machine.reset();
                        ui.close();
                    }
                    ui.separator();
                    // Plugging straight into the port only makes sense with no MPI in the
                    // way — with one installed, cartridges go into its slots instead (below).
                    let direct_port = self.mpi.is_none();
                    if ui
                        .add_enabled(direct_port, egui::Button::new("Insert Cartridge…"))
                        .clicked()
                    {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM Pak", &["rom", "ccc", "bin"])
                            .pick_file()
                        {
                            self.insert_cartridge(path);
                        }
                    }
                    let inserted = direct_port && self.cart_path.is_some();
                    if ui
                        .add_enabled(inserted, egui::Button::new("Eject Cartridge"))
                        .clicked()
                    {
                        self.eject_cartridge();
                        ui.close();
                    }
                    ui.checkbox(&mut self.autostart_cart, "Auto-start cartridge");
                    ui.separator();
                    ui.menu_button("MultiPak Interface", |ui| {
                        let installed = self.mpi.is_some();
                        if ui
                            .add_enabled(!installed, egui::Button::new("Insert MultiPak"))
                            .clicked()
                        {
                            self.insert_multipak();
                            ui.close();
                        }
                        if ui
                            .add_enabled(installed, egui::Button::new("Remove MultiPak"))
                            .clicked()
                        {
                            self.eject_multipak();
                            ui.close();
                        }
                        if installed {
                            ui.separator();
                            for slot in 0..MPI_SLOT_COUNT {
                                let slot_label = match self.mpi.as_ref().map(|m| &m.slots[slot]) {
                                    Some(MPISlot::ROMPak(p)) => format!(
                                        "Slot {} ({})",
                                        slot + 1,
                                        p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                                    ),
                                    Some(MPISlot::FD502) => format!("Slot {} (FD-502)", slot + 1),
                                    Some(MPISlot::DistoRTC) => {
                                        format!("Slot {} (Disto RTC)", slot + 1)
                                    }
                                    _ => format!("Slot {}", slot + 1),
                                };
                                ui.menu_button(slot_label, |ui| {
                                    if ui.button("Insert ROM Pak…").clicked() {
                                        ui.close();
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("ROM Pak", &["rom", "ccc", "bin"])
                                            .pick_file()
                                        {
                                            self.mpi_insert_rompak(slot, path);
                                        }
                                    }
                                    // An FD-502 already installed elsewhere can't also go here
                                    // — the emulated latch only ever models one controller.
                                    let fd502_here = matches!(
                                        self.mpi.as_ref().map(|m| &m.slots[slot]),
                                        Some(MPISlot::FD502)
                                    );
                                    let fd502_elsewhere =
                                        self.machine.bus.cart.as_disk_cart().is_some() && !fd502_here;
                                    if ui
                                        .add_enabled(
                                            !fd502_elsewhere,
                                            egui::Button::new("Insert FD-502"),
                                        )
                                        .clicked()
                                    {
                                        self.mpi_insert_fd502(slot);
                                        ui.close();
                                    }
                                    // Same one-per-machine rule as the FD-502:
                                    // two RTCs would shadow each other at $FF50.
                                    let rtc_here = matches!(
                                        self.mpi.as_ref().map(|m| &m.slots[slot]),
                                        Some(MPISlot::DistoRTC)
                                    );
                                    let rtc_elsewhere = self
                                        .machine
                                        .bus
                                        .cart
                                        .as_disto_rtc()
                                        .is_some()
                                        && !rtc_here;
                                    if ui
                                        .add_enabled(
                                            !rtc_elsewhere,
                                            egui::Button::new("Insert Disto RTC"),
                                        )
                                        .clicked()
                                    {
                                        self.mpi_insert_rtc(slot);
                                        ui.close();
                                    }
                                    let occupied = !matches!(
                                        self.mpi.as_ref().map(|m| &m.slots[slot]),
                                        Some(MPISlot::Empty)
                                    );
                                    if ui
                                        .add_enabled(occupied, egui::Button::new("Eject"))
                                        .clicked()
                                    {
                                        self.mpi_eject_slot(slot);
                                        ui.close();
                                    }
                                });
                            }
                            ui.separator();
                            ui.menu_button("Switch", |ui| {
                                ui.label(
                                    "Selects the power-on SCS/CTS slot. A running program's own \
                                     $FF7F write overrides it until the next reset.",
                                );
                                let current = self.mpi.as_ref().map_or(0, |m| m.switch);
                                for slot in 0..MPI_SLOT_COUNT {
                                    if ui
                                        .selectable_label(current == slot, format!("Slot {}", slot + 1))
                                        .clicked()
                                    {
                                        self.mpi_set_switch(slot);
                                    }
                                }
                            });
                        }
                    });
                    ui.separator();
                    // Disto RTC: directly in the port here, or via a MultiPak
                    // slot submenu above when an MPI is installed.
                    if ui
                        .add_enabled(
                            direct_port && !self.rtc_direct,
                            egui::Button::new("Insert Disto RTC"),
                        )
                        .clicked()
                    {
                        self.insert_rtc();
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.rtc_direct, egui::Button::new("Eject Disto RTC"))
                        .clicked()
                    {
                        self.eject_rtc();
                        ui.close();
                    }
                    let rtc_present = self.machine.bus.cart.as_disto_rtc().is_some();
                    if ui
                        .add_enabled(rtc_present, egui::Button::new("Sync RTC to Host Clock"))
                        .clicked()
                    {
                        self.sync_rtc_to_host();
                        ui.close();
                    }
                    ui.separator();
                    for drive in 0..UI_DRIVES {
                        if ui.button(format!("Insert Disk in Drive {drive}…")).clicked() {
                            ui.close();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Disk image", &["dsk", "jvc", "os9"])
                                .pick_file()
                            {
                                self.request_insert_disk(drive, path);
                            }
                        }
                        if ui.button(format!("New Blank Disk in Drive {drive}…")).clicked() {
                            ui.close();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Disk image", &["dsk"])
                                .set_file_name("untitled.dsk")
                                .save_file()
                            {
                                self.request_new_blank_disk(drive, path);
                            }
                        }
                        let label = match &self.disk_paths[drive] {
                            Some(p) => format!(
                                "Eject Drive {drive} ({})",
                                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                            ),
                            None => format!("Eject Drive {drive}"),
                        };
                        let mounted = self.disk_paths[drive].is_some();
                        if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                            self.eject_disk(drive);
                            ui.close();
                        }
                    }
                    ui.separator();
                    for drive in 0..UI_DRIVES {
                        if ui.button(format!("Insert VHD {drive}…")).clicked() {
                            ui.close();
                            if let Some(path) =
                                rfd::FileDialog::new().add_filter("VHD image", &["vhd"]).pick_file()
                            {
                                self.insert_vhd(drive, path);
                            }
                        }
                        let label = match &self.vhd_paths[drive] {
                            Some(p) => format!(
                                "Eject VHD {drive} ({})",
                                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                            ),
                            None => format!("Eject VHD {drive}"),
                        };
                        let mounted = self.vhd_paths[drive].is_some();
                        if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                            self.eject_vhd(drive);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Insert Tape…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Cassette image", &["cas", "wav"])
                            .pick_file()
                        {
                            self.insert_tape(path);
                        }
                    }
                    if ui.button("New Tape…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Cassette image", &["cas"])
                            .set_file_name("untitled.cas")
                            .save_file()
                        {
                            self.new_tape(path);
                        }
                    }
                    let tape_mounted = self.tape_path.is_some();
                    if ui
                        .add_enabled(tape_mounted, egui::Button::new("Rewind Tape"))
                        .clicked()
                    {
                        self.machine.bus.cassette.rewind();
                        ui.close();
                    }
                    let label = match &self.tape_path {
                        Some(p) => format!(
                            "Eject Tape ({})",
                            p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                        ),
                        None => "Eject Tape".to_string(),
                    };
                    if ui.add_enabled(tape_mounted, egui::Button::new(label)).clicked() {
                        self.eject_tape();
                        ui.close();
                    }
                    ui.checkbox(&mut self.save_tape_wav, "Also save tape audio (.wav)");
                    ui.separator();
                    let capturing = self.print_capture_path.is_some();
                    if ui
                        .add_enabled(!capturing, egui::Button::new("Start Print Capture…"))
                        .clicked()
                    {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Text file", &["txt"])
                            .set_file_name("printout.txt")
                            .save_file()
                        {
                            self.start_print_capture(path);
                        }
                    }
                    let label = match &self.print_capture_path {
                        Some(p) => format!(
                            "Stop Print Capture ({})",
                            p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                        ),
                        None => "Stop Print Capture".to_string(),
                    };
                    if ui
                        .add_enabled(capturing, egui::Button::new(label))
                        .clicked()
                    {
                        self.stop_print_capture();
                        ui.close();
                    }
                    ui.checkbox(&mut self.print_capture_lf, "Translate CR to LF")
                        .on_hover_text(
                            "Rewrite the CoCo's CR line endings as LF so the capture reads as \
                             normal text. Takes effect when a capture starts.",
                        );
                });
                ui.menu_button("Keyboard", |ui| {
                    for mode in [KbMode::Positional, KbMode::Symbolic] {
                        if ui.selectable_label(self.kb_mode == mode, mode.label()).clicked() {
                            self.set_mode(mode);
                        }
                    }
                    ui.separator();
                    if ui.button("Key layout (F10)").clicked() {
                        self.show_kbd_help = !self.show_kbd_help;
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
                    ui.separator();
                    let mut paper_open = self.paper_window.open;
                    if ui.checkbox(&mut paper_open, "Printer Paper").changed() {
                        self.toggle_paper_window();
                    }
                    ui.separator();
                    // Swapping the monitor cable doesn't erase machine state,
                    // so this takes effect live rather than requiring a
                    // power cycle.
                    for (mt, label) in [
                        (MonitorType::RGB, "RGB monitor"),
                        (MonitorType::Composite, "Composite monitor"),
                    ] {
                        if ui
                            .selectable_label(self.machine.bus.gime.monitor == mt, label)
                            .clicked()
                        {
                            self.machine.bus.gime.monitor = mt;
                        }
                    }
                });
                ui.menu_button("Joysticks", |ui| self.joysticks.menu_ui(ui));
                ui.menu_button("Sound", |ui| self.audio.menu_ui(ui));
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = !self.show_about;
                        ui.close();
                    }
                });
            });
        });

        // Toolbar: one-click access to the most frequent actions, redundant with
        // (but quicker than) the menu bar above.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let run_label = if self.running { "Pause" } else { "Run" };
                if ui.button(run_label).clicked() {
                    self.running = !self.running;
                }
                if ui.button("Reset").clicked() {
                    self.machine.reset();
                }
                ui.separator();
                if ui.button("⌨ Keys (F10)").clicked() {
                    self.show_kbd_help = !self.show_kbd_help;
                }
                ui.checkbox(&mut self.aspect_correct, "4:3 (F9)");
            });
        });

        // Status bar: read-only live state, no controls.
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(if self.running { "Running" } else { "Paused" });
                ui.separator();
                ui.label(format!("Keyboard: {} (F12)", self.kb_mode.label()));
                if let Some(path) = &self.cart_path {
                    ui.separator();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    ui.label(format!("Cart: {name}"));
                }
                if let Some(mpi) = &self.mpi {
                    ui.separator();
                    let slots: Vec<String> = mpi
                        .slots
                        .iter()
                        .enumerate()
                        .map(|(i, slot)| {
                            let label = match slot {
                                MPISlot::Empty => "-".to_string(),
                                MPISlot::ROMPak(p) => {
                                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string()
                                }
                                MPISlot::FD502 => "FD-502".to_string(),
                                MPISlot::DistoRTC => "RTC".to_string(),
                            };
                            format!("S{}:{label}", i + 1)
                        })
                        .collect();
                    ui.label(format!("MPI [{}]", slots.join(" ")));
                }
                for drive in 0..UI_DRIVES {
                    let Some(path) = &self.disk_paths[drive] else {
                        continue;
                    };
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    // "*" = modified in memory; written back on eject/exit.
                    let dirty = self
                        .machine
                        .bus
                        .cart
                        .as_disk_cart()
                        .and_then(|c| c.disk(drive))
                        .is_some_and(|d| d.dirty());
                    let active = self
                        .machine
                        .bus
                        .cart
                        .as_disk_cart()
                        .is_some_and(|c| c.drive_active(drive));
                    ui.separator();
                    drive_activity_light(ui, active);
                    ui.label(format!("D{drive}: {name}{}", if dirty { "*" } else { "" }));
                }
                for drive in 0..UI_DRIVES {
                    let Some(path) = &self.vhd_paths[drive] else {
                        continue;
                    };
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    ui.separator();
                    ui.label(format!("VHD{drive}: {name}"));
                }
                if let Some(path) = &self.tape_path {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    let cassette = &self.machine.bus.cassette;
                    // The icon reddens while the motor runs (relay closed —
                    // CLOAD/CSAVE/MOTOR ON); the counter is the playback
                    // position in tape bytes; "*" as for floppies.
                    let motor = self.machine.bus.pia1.a.c2_output();
                    let (pos, len) = cassette.position();
                    ui.separator();
                    cassette_activity_light(ui, motor);
                    ui.label(format!(
                        "Tape: {name}{} [{pos}/{len}]",
                        if cassette.dirty() { "*" } else { "" }
                    ));
                }
            });
        });

        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if let new_vm::NewVmAction::Create(spec) = self.new_vm.show(ctx) {
            match self.create_vm(spec.config, ctx) {
                Ok(()) => {
                    // The machine booted; cartridge/media problems (e.g.
                    // missing disk11.rom, unreadable image) are reported
                    // like a menu insert, not as a create failure.
                    // Blank(None) is the manager flow's auto-placed
                    // spelling and can't be produced here.
                    if spec.cartridge == new_vm::CartridgeChoice::FD502 {
                        if let Err(e) = self.ensure_disk_controller() {
                            self.cart_error = Some(e);
                        } else {
                            for (drive, choice) in spec.disks.into_iter().enumerate() {
                                match choice {
                                    new_vm::MediaChoice::File(path) => {
                                        self.insert_disk(drive, path)
                                    }
                                    new_vm::MediaChoice::Blank(Some(path)) => {
                                        self.new_blank_disk(drive, path)
                                    }
                                    new_vm::MediaChoice::None
                                    | new_vm::MediaChoice::Blank(None) => {}
                                }
                            }
                        }
                    }
                    match spec.tape {
                        new_vm::MediaChoice::File(path) => self.insert_tape(path),
                        new_vm::MediaChoice::Blank(Some(path)) => self.new_tape(path),
                        new_vm::MediaChoice::None | new_vm::MediaChoice::Blank(None) => {}
                    }
                    self.new_vm.close();
                }
                Err(e) => self.new_vm.error = Some(e),
            }
        }
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        if self.pending_disk_action.is_some() {
            // Match the dialog body to the button font (egui's default body
            // text is a touch smaller) and give the text room.
            let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
            const DIALOG_MARGIN: i8 = 16;
            egui::Window::new(window_title(ctx, "Insert disk controller?"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(
                                "The FD-502 disk controller isn't installed yet. Installing \
                                 it swaps the cartridge and cold-restarts the machine — any \
                                 unsaved work in memory will be lost.",
                            )
                            .size(font),
                        );
                        ui.add_space(DIALOG_MARGIN as f32);
                        ui.horizontal(|ui| {
                            // Roomier buttons: pad text away from the button edge.
                            ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                            if ui.button("Insert & Restart").clicked() {
                                match self.pending_disk_action.take() {
                                    Some(PendingDiskAction::Insert { drive, path }) => {
                                        self.insert_disk(drive, path)
                                    }
                                    Some(PendingDiskAction::NewBlank { drive, path }) => {
                                        self.new_blank_disk(drive, path)
                                    }
                                    None => {}
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.pending_disk_action = None;
                            }
                        });
                    });
                });
        }

        if let Some(err) = self.cart_error.clone() {
            let mut open = true;
            egui::Window::new(window_title(ctx, "Cartridge Error"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(err);
                    if ui.button("OK").clicked() {
                        self.cart_error = None;
                    }
                });
            if !open {
                self.cart_error = None;
            }
        }
    }

    /// The CoCo display itself: the letterboxed, (optionally) aspect-
    /// corrected framebuffer texture, filling whatever `ui` it's given.
    /// Split out of [`Self::window_ui`]'s `CentralPanel` closure so the
    /// manager's `ViewportClass::Embedded` fallback can show just this —
    /// without the rest of [`Self::draw_chrome`] — inside a plain
    /// `egui::Window` instead of a full-window `CentralPanel`
    /// (`docs/plan-machine-persistence.md` "one native window per running
    /// VM"). Requires [`Self::step_emulation`] to have already run this
    /// frame (it uploads `self.texture`, `unwrap`ped below).
    fn draw_display(&mut self, ui: &mut egui::Ui) {
        let tex = self.texture.as_ref().unwrap();
        let tex_size = tex.size_vec2();
        // Aspect the displayed frame should have, independent of the buffer's
        // pixel dimensions: 4:3 when corrected, else the raw square-pixel aspect.
        // This keeps the frontend mode-agnostic — any renderer's buffer size fits.
        let aspect = if self.aspect_correct {
            TARGET_ASPECT
        } else {
            tex_size.x / tex_size.y
        };
        // Largest rect of that aspect that fits the panel, centered (letterboxed).
        let avail = ui.available_rect_before_wrap();
        let mut w = avail.width();
        let mut h = w / aspect;
        if h > avail.height() {
            h = avail.height();
            w = h * aspect;
        }
        let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
        let sized = egui::load::SizedTexture::new(tex.id(), rect.size());
        ui.put(rect, egui::Image::new(sized));
        // Remembered for `drive_joysticks` next frame, to map pointer
        // position to joystick axes (see the `display_rect` field doc).
        self.display_rect = rect;
    }

    /// The full app window for one frame: emulation step, every menu/toolbar/
    /// dialog, then the display, in that order — exactly the body
    /// `eframe::App::update` ran before this method existed. `pub(crate)` so
    /// the manager's per-VM immediate viewport (`manager.rs`'s
    /// `draw_running_vms`, `ViewportClass::Default`/native case) can call it
    /// directly on a VM it owns, reproducing the direct-boot window's full
    /// chrome inside its own native OS window
    /// (`docs/plan-machine-persistence.md` "one native window per running
    /// VM"). The trait method below (kept for the direct-boot CLI path,
    /// which stays byte-for-byte identical) just forwards here.
    pub(crate) fn window_ui(&mut self, ctx: &egui::Context) {
        self.step_emulation(ctx);
        self.draw_chrome(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| self.draw_display(ui));
    }
}

impl eframe::App for CocoApp {
    /// Write modified floppies and tape back to their files on quit — a BASIC
    /// `SAVE`/`CSAVE` only exists in the in-memory image until then.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_media();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.window_ui(ctx);
    }
}

/// Positional map: host physical key → CoCo matrix position (MAME's layout).
fn key_to_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        // Letters: @ A..Z run linearly from (0,0).
        K::A => (0, 1), K::B => (0, 2), K::C => (0, 3), K::D => (0, 4),
        K::E => (0, 5), K::F => (0, 6), K::G => (0, 7),
        K::H => (1, 0), K::I => (1, 1), K::J => (1, 2), K::K => (1, 3),
        K::L => (1, 4), K::M => (1, 5), K::N => (1, 6), K::O => (1, 7),
        K::P => (2, 0), K::Q => (2, 1), K::R => (2, 2), K::S => (2, 3),
        K::T => (2, 4), K::U => (2, 5), K::V => (2, 6), K::W => (2, 7),
        K::X => (3, 0), K::Y => (3, 1), K::Z => (3, 2),
        // Digits.
        K::Num0 => (4, 0), K::Num1 => (4, 1), K::Num2 => (4, 2), K::Num3 => (4, 3),
        K::Num4 => (4, 4), K::Num5 => (4, 5), K::Num6 => (4, 6), K::Num7 => (4, 7),
        K::Num8 => (5, 0), K::Num9 => (5, 1),
        // Punctuation (host physical key → CoCo key at that position, per MAME).
        K::Minus => (5, 2),      // CoCo ':'
        K::Semicolon => (5, 3),  // CoCo ';'
        K::Comma => (5, 4),      // CoCo ','
        K::Equals => (5, 5),     // CoCo '-'
        K::Period => (5, 6),     // CoCo '.'
        K::Slash => (5, 7),      // CoCo '/'
        K::OpenBracket => kbd::AT,
        // Movement / control.
        K::Space => kbd::SPACE,
        K::Enter => kbd::ENTER,
        K::Backspace => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowLeft => kbd::LEFT,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
}

/// Control keys that symbolic mode still routes positionally (they produce no text).
fn control_key_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        K::Enter => kbd::ENTER,
        K::Backspace | K::ArrowLeft => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
}

/// Keys claimed by `joy::JoySource::Keys` (arrows for the axes, Z/X for the fire
/// buttons) once a joystick port uses that source — these stop reaching the CoCo
/// keyboard matrix so the two consumers don't fight over the same physical keys.
fn is_joystick_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::Z
            | egui::Key::X
    )
}

// `MachineVariant`/`MemorySize`/`VideoStandard` are all foreign types (defined
// in `coco-core`), so none of them can derive `clap::ValueEnum` here (orphan
// rule) without pulling a `clap` dependency into the core crate. Each gets a
// plain string `value_parser` function instead — same shape, no mirror enum
// (`docs/coco12-plan.md` Phase 5).

/// `clap` value parser for `--machine`.
fn parse_machine(s: &str) -> Result<MachineVariant, String> {
    match s {
        "coco1" => Ok(MachineVariant::Coco1),
        "coco2" => Ok(MachineVariant::Coco2),
        "coco3" => Ok(MachineVariant::Coco3),
        _ => Err(format!(
            "unknown machine '{s}' (expected coco1, coco2, or coco3)"
        )),
    }
}

/// Short label for the window title.
const fn machine_label(variant: MachineVariant) -> &'static str {
    match variant {
        MachineVariant::Coco1 => "CoCo 1",
        MachineVariant::Coco2 => "CoCo 2",
        MachineVariant::Coco3 => "CoCo 3",
    }
}

/// `clap` value parser for `--ram`. Accepts every [`MemorySize`] spelling
/// across both machine families (`docs/coco12-plan.md`) —
/// [`MachineConfig::validate`] rejects the wrong family for the chosen
/// `--machine`.
fn parse_ram(s: &str) -> Result<MemorySize, String> {
    match s {
        "4k" => Ok(MemorySize::K4),
        "16k" => Ok(MemorySize::K16),
        "32k" => Ok(MemorySize::K32),
        "64k" => Ok(MemorySize::K64),
        "128k" => Ok(MemorySize::K128),
        "512k" => Ok(MemorySize::K512),
        "2048k" => Ok(MemorySize::K2048),
        _ => Err(format!(
            "unknown RAM size '{s}' (expected 4k, 16k, 32k, 64k, 128k, 512k, or 2048k)"
        )),
    }
}

/// `clap` value parser for `--video`.
fn parse_video(s: &str) -> Result<VideoStandard, String> {
    match s {
        "ntsc" => Ok(VideoStandard::NTSC),
        "pal" => Ok(VideoStandard::PAL),
        _ => Err(format!(
            "unknown video standard '{s}' (expected ntsc or pal)"
        )),
    }
}

/// Composite vs RGB monitor cable. Mirrors [`MonitorType`].
#[derive(Clone, Copy, ValueEnum)]
enum MonitorArg {
    RGB,
    #[value(name = "cmp", alias = "composite")]
    Composite,
}

impl From<MonitorArg> for MonitorType {
    fn from(m: MonitorArg) -> Self {
        match m {
            MonitorArg::RGB => MonitorType::RGB,
            MonitorArg::Composite => MonitorType::Composite,
        }
    }
}

#[derive(Parser)]
#[command(name = "coco", version, about = "A Tandy Color Computer emulator")]
struct Cli {
    /// Which machine to emulate (coco1, coco2, coco3).
    #[arg(long, default_value = "coco3", value_parser = parse_machine)]
    machine: MachineVariant,

    /// Boot ROM image. Defaults, per `--machine`, to `roms/coco3.rom` (CoCo
    /// 3) or a flat image composed from `roms/bas1{0,1,2,3}.rom` +
    /// `roms/extbas1{0,1}.rom` (CoCo 1/2 — see `docs/coco12-plan.md`). When
    /// given explicitly for CoCo 1/2, must already be that same pre-composed
    /// flat layout (extbas at offset 0, Color BASIC at offset $2000).
    #[arg(long, value_name = "PATH")]
    rom: Option<PathBuf>,

    /// Cartridge ROM pak to insert at boot (`.rom`/`.ccc`/`.bin`). Without
    /// --mpi this plugs directly into the cartridge port (conflicts with
    /// --disk0/--disk1/--fd502, which also want that port); with --mpi it
    /// goes into slot 1 instead, alongside the FD-502 in slot 4.
    #[arg(long, value_name = "PATH")]
    cart: Option<PathBuf>,

    /// Floppy image for drive 0 (`.dsk`/`.jvc`/`.os9`); implies the FD-502
    /// disk controller (`roms/disk11.rom`), in the cartridge slot directly or
    /// (with --mpi) in slot 4.
    #[arg(long, value_name = "PATH")]
    disk0: Option<PathBuf>,

    /// Floppy image for drive 1 (see `--disk0`).
    #[arg(long, value_name = "PATH")]
    disk1: Option<PathBuf>,

    /// VHD (virtual hard disk) image for drive 0, for NitrOS-9's `emudsk`
    /// driver. A bus-level device ($FF80-$FF86) independent of the cartridge
    /// slot, so unlike --disk0/--disk1 this doesn't conflict with --cart.
    #[arg(long, value_name = "PATH")]
    vhd0: Option<PathBuf>,

    /// VHD image for drive 1 (see `--vhd0`).
    #[arg(long, value_name = "PATH")]
    vhd1: Option<PathBuf>,

    /// Insert the FD-502 disk controller with empty drives, so Disk BASIC
    /// boots and blank disks can be added (and DSKINI'd) from the menu.
    /// Implied by --disk0/--disk1.
    #[arg(long, default_value_t = false)]
    fd502: bool,

    /// Insert a 4-slot Tandy Multi-Pak Interface into the cartridge port
    /// instead of plugging --cart/--disk*/--fd502/--rtc directly into it:
    /// --cart goes into slot 1, the FD-502 (implied by --disk0/--disk1/
    /// --fd502) into slot 4, and the RTC into slot 3 — the conventional
    /// real-world layout (also MAME's default), letting a cartridge, the
    /// disk controller, and the clock coexist.
    #[arg(long, default_value_t = false)]
    mpi: bool,

    /// Insert a Disto real-time clock (OKI MSM6242 at $FF50-$FF53, for
    /// NitrOS-9's clock2_disto drivers), running on the host's local clock —
    /// directly in the cartridge port, or (with --mpi) in slot 3.
    #[arg(long, default_value_t = false)]
    rtc: bool,

    /// Installed RAM (4k, 16k, 32k, 64k, 128k, 512k, 2048k). Defaults, per
    /// `--machine`, to 512K (CoCo 3) or 64K (CoCo 1/2).
    #[arg(long, value_parser = parse_ram)]
    ram: Option<MemorySize>,

    /// Master video standard (crystal), independent of the GIME 50/60 Hz mode
    /// bit (ntsc or pal).
    #[arg(long, default_value = "ntsc", value_parser = parse_video)]
    video: VideoStandard,

    /// Composite vs RGB monitor cable (CoCo 3 only — a CoCo 1/2 has no
    /// monitor port, just RF out to a TV). Real hardware drives both
    /// signals simultaneously; this picks which one the emulated monitor
    /// decodes (also toggleable live from the View menu). Defaults to RGB
    /// on a CoCo 3.
    #[arg(long, value_enum)]
    monitor: Option<MonitorArg>,

    /// Also save a `.wav` of the tape audio alongside the canonical `.cas`
    /// on every tape write-back (see the "Also save tape audio (.wav)"
    /// Machine-menu checkbox, which this just sets the initial value of).
    #[arg(long, default_value_t = false)]
    tape_wav: bool,

    /// Start "print to text file" capture at this path as soon as the
    /// machine boots (create/truncate — see the Machine menu's "Start Print
    /// Capture…", which this is the CLI equivalent of).
    #[arg(long, value_name = "PATH")]
    print_capture: Option<PathBuf>,
}

/// Read an explicit `--rom` image as-is: a CoCo 3 image, or — for CoCo 1/2 —
/// an already pre-composed flat layout (see the `Cli::rom` doc).
fn load_explicit_rom(path: &Path) -> Result<Box<[u8]>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            report_rom_validation(path, &bytes);
            Ok(bytes.into_boxed_slice())
        }
        Err(e) => Err(format!("could not load {}: {e}", path.display())),
    }
}

/// Load the default boot ROM set for `variant` from `roms_dir` (copyrighted
/// and git-ignored, `./roms`): `coco3.rom` for the CoCo 3, or a flat image
/// composed from the newest Color/Extended BASIC dumps present for CoCo 1/2
/// ([`compose_coco12_rom`]). Failures are returned rather than fatal because
/// the "New…" dialog shows them inline; `main` prints them and exits.
fn load_default_rom(variant: MachineVariant, roms_dir: &Path) -> Result<Box<[u8]>, String> {
    match variant {
        MachineVariant::Coco3 => {
            let path = roms_dir.join("coco3.rom");
            match std::fs::read(&path) {
                Ok(bytes) => {
                    report_rom_validation(&path, &bytes);
                    Ok(bytes.into_boxed_slice())
                }
                Err(e) => Err(format!("could not load {}: {e}", path.display())),
            }
        }
        MachineVariant::Coco1 | MachineVariant::Coco2 => match compose_coco12_rom(roms_dir) {
            Coco12RomResult::Composed { image, bas, extbas } => {
                report_rom_validation(&bas.0, &bas.1);
                match extbas {
                    Some((ext_path, ext_bytes)) => report_rom_validation(&ext_path, &ext_bytes),
                    None => tracing::info!(
                        "no Extended Color BASIC ROM found ({}); booting Color BASIC only",
                        EXTENDED_BASIC_CANDIDATES.join(", ")
                    ),
                }
                Ok(image)
            }
            Coco12RomResult::NoColorBasic => Err(format!(
                "no Color BASIC ROM found: place one of {} in {}",
                COCO_BASIC_CANDIDATES.join(", "),
                roms_dir.display()
            )),
        },
    }
}

/// Build a running [`CocoApp`] from a saved machine definition
/// (`machine_def::MachineDef`): the same steps `main()`'s CLI branch below
/// performs — load the ROM ([`load_explicit_rom`] for an explicit
/// `[hardware].rom`, else [`load_default_rom`] with the same `./roms`
/// resolution the CLI path uses), mount `[media]` (cart/disks/vhds/tape,
/// resolved with `machine_def::resolve_media_path`) and `[peripherals]`
/// (MPI/RTC) with the same `CocoApp` methods and ordering, and enforce the
/// same single-cartridge-port rule — but every failure is a returned `Err`
/// here instead of a process exit, since the caller (the manager's Start
/// button, `manager.rs`) must show it in the detail pane rather than crash
/// the whole app (`docs/plan-machine-persistence.md` step 5). On any
/// mount-time failure (a bad disk/VHD/cassette image, or a disk-BASIC ROM
/// read failure inside `mpi_insert_fd502` — not just a missing path, caught
/// early below) the partially-built VM is discarded rather than returned:
/// callers get either a fully-mounted machine or a precise error, never a
/// half-broken one.
pub(crate) fn launch_machine(def: &machine_def::MachineDef, slug: &str) -> Result<CocoApp, String> {
    let config = def.to_machine_config()?;

    let rom = match &def.hardware.rom {
        Some(path) => load_explicit_rom(Path::new(path)),
        None => {
            let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
            load_default_rom(config.variant, &roms_dir)
        }
    }?;

    let cart_path = def
        .media
        .cart
        .as_deref()
        .map(|p| machine_def::resolve_media_path(p, slug));
    let disk_paths = [
        def.media.disk0.as_deref().map(|p| machine_def::resolve_media_path(p, slug)),
        def.media.disk1.as_deref().map(|p| machine_def::resolve_media_path(p, slug)),
    ];
    let vhd_paths = [
        def.media.vhd0.as_deref().map(|p| machine_def::resolve_media_path(p, slug)),
        def.media.vhd1.as_deref().map(|p| machine_def::resolve_media_path(p, slug)),
    ];
    let tape_path = def.media.tape.as_deref().map(|p| machine_def::resolve_media_path(p, slug));

    // Same rule `main()`'s CLI branch enforces by hand (clap's declarative
    // `conflicts_with` can't express "only when --mpi is absent"): cart,
    // disk0/disk1 (which imply the FD-502), and rtc all want the single
    // cartridge port unless an MPI is installed.
    let mpi = def.peripherals.mpi;
    let rtc = def.peripherals.rtc;
    // Disk media implies the controller even when the flag is off (older
    // definition files predate `[peripherals].fd502`).
    let fd502 = def.peripherals.fd502 || disk_paths[0].is_some() || disk_paths[1].is_some();
    let port_claims = [cart_path.is_some(), fd502, rtc]
    .into_iter()
    .filter(|&claims| claims)
    .count();
    if !mpi && port_claims > 1 {
        return Err(
            "cart, disk0/disk1, and rtc all need the cartridge port; enable the MultiPak \
             Interface peripheral to combine them"
                .to_string(),
        );
    }

    // No definition field for this UI preference yet (`machine_def.rs`'s
    // schema doc); matches the CLI's own `--tape-wav` default of off.
    let save_tape_wav = false;
    let mut app = if mpi {
        CocoApp::new(config, rom, None, [None, None], vhd_paths, save_tape_wav)
    } else {
        CocoApp::new(config, rom, cart_path.clone(), disk_paths.clone(), vhd_paths, save_tape_wav)
    };

    if mpi {
        app.insert_multipak();
        if let Some(path) = cart_path {
            app.mpi_insert_rompak(0, path);
        }
        if fd502 {
            app.mpi_insert_fd502(MPI_SLOT_COUNT - 1);
        }
        if rtc {
            app.mpi_insert_rtc(DEFAULT_RTC_SLOT);
        }
        for (drive, path) in disk_paths.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_disk(drive, path);
            }
        }
    } else if rtc {
        // cart/fd502 (disk media is handled by the `CocoApp::new` call
        // above, same as the CLI's non-mpi branch) and rtc are mutually
        // exclusive here — `port_claims` already rejected any combination
        // of them without an MPI.
        app.insert_rtc();
    } else if fd502 && let Err(e) = app.ensure_disk_controller() {
        // Empty-drive FD-502 from `[peripherals].fd502` alone; with disk
        // media set, `CocoApp::new` already inserted the controller and
        // this is a no-op Ok.
        app.cart_error = Some(e);
    }

    if let Some(path) = tape_path {
        app.insert_tape(path);
    }

    // Every `insert_*`/`mpi_insert_*` helper above records its own failure
    // in `cart_error` rather than returning a `Result` (it's designed to run
    // from a live menu click, where the machine keeps running and a dialog
    // reports the problem). Promote that here into the launch `Result`
    // instead of returning a VM with a swallowed error nobody's watching
    // for yet.
    if let Some(err) = app.cart_error.take() {
        return Err(err);
    }

    // The definition's [ui] preferences are the launched window's *starting*
    // state; F9 (aspect) and F12 (keyboard mode) keep working as live
    // toggles afterwards — the file controls where they begin, exactly like
    // the hardware section controls the machine's construction.
    app.aspect_correct = def.ui.aspect_correct;
    app.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => KbMode::Symbolic,
    };

    Ok(app)
}

/// Plain-SAM ROM composition (CoCo 1/2 only): the flat image `bus.rs`'s
/// primary-SAM path expects is Extended Color BASIC at offset 0 (8K), Color
/// BASIC at offset [`COCO12_BAS_OFFSET`] (8K) — `docs/coco12-plan.md` "ROM
/// files"; `bus.rs::SAM_BAS_ROM_OFFSET`.
const COCO12_BAS_OFFSET: usize = 8 * 1024;
/// Color BASIC dumps accepted for `--machine coco1`/`coco2` (any one is
/// enough to boot), newest-preferred among the versions these machines
/// actually shipped with: 1.2 first, down to 1.0. `bas13.rom` (the CoCo 2B's
/// Color BASIC, shipped with the MC6847T1 boards) boots fine too but is the
/// far rarer dump, so it stays a last resort rather than the preferred one —
/// even though the CoCo 2 now defaults to `VdgVariant::Mc6847T1`, 1.2 runs
/// identically on a T1 machine (lowercase just goes unused).
const COCO_BASIC_CANDIDATES: &[&str] = &["bas12.rom", "bas11.rom", "bas10.rom", "bas13.rom"];
/// Newest-preferred Extended Color BASIC dumps; optional
/// (`docs/coco12-plan.md` "ROM files": a Color-BASIC-only machine still
/// boots).
const EXTENDED_BASIC_CANDIDATES: &[&str] = &["extbas11.rom", "extbas10.rom"];
/// Fill byte for the Extended Color BASIC half of the flat image when no
/// Extended BASIC dump is present — the conventional open-bus value used
/// elsewhere in the emulator (`docs/coco12-plan.md`).
const OPEN_BUS_FILLER: u8 = 0xFF;

/// Find the first of `candidates` that exists under `roms_dir`, returning its
/// path and contents.
fn find_rom(roms_dir: &Path, candidates: &[&str]) -> Option<(PathBuf, Vec<u8>)> {
    candidates.iter().find_map(|name| {
        let path = roms_dir.join(name);
        std::fs::read(&path).ok().map(|bytes| (path, bytes))
    })
}

/// What [`compose_coco12_rom`] found (or didn't) while composing the flat
/// image, so the CLI-facing caller can report it and the pure composition
/// logic stays unit-testable without touching `std::process::exit`.
enum Coco12RomResult {
    Composed {
        image: Box<[u8]>,
        bas: (PathBuf, Vec<u8>),
        extbas: Option<(PathBuf, Vec<u8>)>,
    },
    /// No Color BASIC dump found under `roms_dir` — nothing to boot.
    NoColorBasic,
}

/// Search `roms_dir` for the newest-present Color BASIC dump (required) and
/// Extended Color BASIC dump (optional) and lay them out the way `bus.rs`'s
/// plain-SAM decode expects. Missing Extended BASIC leaves that half of the
/// image at [`OPEN_BUS_FILLER`] rather than failing (`docs/coco12-plan.md`
/// "ROM files": a Color-BASIC-only machine still boots). Pure (no I/O side
/// effects beyond reading `roms_dir`, no process exit) so it's unit-testable.
fn compose_coco12_rom(roms_dir: &Path) -> Coco12RomResult {
    let Some((bas_path, bas_bytes)) = find_rom(roms_dir, COCO_BASIC_CANDIDATES) else {
        return Coco12RomResult::NoColorBasic;
    };

    let mut image = vec![OPEN_BUS_FILLER; COCO12_BAS_OFFSET];
    let extbas = find_rom(roms_dir, EXTENDED_BASIC_CANDIDATES);
    if let Some((_, ext_bytes)) = &extbas {
        let n = ext_bytes.len().min(COCO12_BAS_OFFSET);
        image[..n].copy_from_slice(&ext_bytes[..n]);
    }
    image.extend_from_slice(&bas_bytes);
    Coco12RomResult::Composed {
        image: image.into_boxed_slice(),
        bas: (bas_path, bas_bytes),
        extbas,
    }
}

/// Per-variant default RAM size when `--ram` isn't given
/// (`docs/coco12-plan.md` Phase 5).
fn default_ram(variant: MachineVariant) -> MemorySize {
    match variant {
        MachineVariant::Coco3 => MemorySize::K512,
        MachineVariant::Coco1 | MachineVariant::Coco2 => MemorySize::K64,
    }
}

/// Per-variant default VDG chip when no explicit choice is made: the T1
/// (CoCo 2B) on a CoCo 2, the plain MC6847 on a CoCo 1 (the only choice
/// `MachineConfig::validate` accepts there), and `None` on a CoCo 3, which
/// has no VDG at all. Shared by the CLI path below, `new_vm.rs`'s
/// `constrain`, and `machine_def.rs`'s `to_machine_config`.
const fn default_vdg(variant: MachineVariant) -> Option<VDGVariant> {
    match variant {
        MachineVariant::Coco2 => Some(VDGVariant::MC6847T1),
        MachineVariant::Coco1 => Some(VDGVariant::MC6847),
        MachineVariant::Coco3 => None,
    }
}

/// One advisory log line per loaded system ROM, checked against the
/// MAME-derived manifest ([`coco_core::rom_db`]). Never fatal: patched and
/// homebrew images are legitimate, but a corrupt known dump should say so.
fn report_rom_validation(path: &Path, bytes: &[u8]) {
    use coco_core::rom_db::{self, Validation};
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match rom_db::validate(&name, bytes) {
        Validation::Verified(known) => {
            tracing::info!("{name}: verified {} [crc32 {:08x}]", known.desc, known.crc32);
        }
        Validation::Mismatch { expected, actual_crc32, actual_size } => {
            tracing::warn!(
                "{name} does not match the known dump of {}: \
                 expected {} bytes crc32 {:08x}, got {} bytes crc32 {actual_crc32:08x} \
                 (patched image, or a bad dump)",
                expected.desc, expected.size, expected.crc32, actual_size,
            );
        }
        Validation::Unknown => {
            tracing::info!(
                "{name} is not in the known-ROM manifest ({} bytes, crc32 {:08x})",
                bytes.len(),
                rom_db::crc32(bytes),
            );
        }
    }
}

fn setup_logging() {
    // Legacy Windows conhost only interprets VT escape codes after the app
    // opts in; a no-op everywhere else. On failure, fall back to plain text.
    let vt_ok = enable_ansi_support::enable_ansi_support().is_ok();
    let use_color = vt_ok && std::io::IsTerminal::is_terminal(&std::io::stdout());
    // Leveled stdout logging, colored only when stdout is a terminal.
    // `RUST_LOG` filters per module (e.g. `RUST_LOG=info,eframe=warn` or
    // `RUST_LOG=coco_egui::audio=debug`); without it, only `warn` and above
    // is shown.
    tracing_subscriber::fmt()
        .with_ansi(use_color)
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::WARN.into())
                .from_env_lossy(),
        )
        .init();
}

fn banner() {
    let sep = "─".repeat(76);
    println!("{}{}{}\n{} CoCoVM v{} {} A Tandy {}{}{} Color Computers emulator {} © 2026 Éric Spérano {}\n{}{}{}",
            "╭".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             sep.if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "╮".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "│".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             env!("CARGO_PKG_VERSION").if_supports_color(Stream::Stdout, |v| v.cyan()),
             "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::BittersweetOrange>()),
             "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::PersianGreen>()),
             "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::ScampiIndigo>()),
             "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "│".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "╰".if_supports_color(Stream::Stdout, |v| v.dimmed()),
             sep.if_supports_color(Stream::Stdout, |v| v.dimmed()),
             "╯".if_supports_color(Stream::Stdout, |v| v.dimmed()),
    );
}

const ASSETS_URL: &str = "https://assets.spe.quebec/cocovm-assets-v2.tgz";

/// Whether `dir` exists and contains at least one entry.
fn dir_has_files(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// Unpack a gzipped tar stream into `dest`. Split from the download so the
/// extraction can be unit-tested without a network.
fn unpack_assets(reader: impl std::io::Read, dest: &Path) -> std::io::Result<()> {
    let gz = flate2::read::GzDecoder::new(reader);
    tar::Archive::new(gz).unpack(dest)
}

/// Download [`ASSETS_URL`] and unpack it into `dest`, streaming — the
/// tarball is never held in memory or written to disk whole.
fn download_and_unpack_assets(dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dest)?;
    let response = ureq::get(ASSETS_URL).call()?;
    unpack_assets(response.into_body().into_reader(), dest)?;
    Ok(())
}

fn ensure_assets() {
    let Some(data_dir) = paths::data_dir() else {
        eprintln!("no home directory found; cannot locate the asset directories");
        std::process::exit(1);
    };
    let missing: Vec<String> = [paths::roms_dir(), paths::images_dir()]
        .into_iter()
        .flatten()
        .filter(|dir| !dir_has_files(dir))
        .map(|dir| dir.display().to_string())
        .collect();
    if missing.is_empty() {
        return;
    }
    println!("Downloading {ASSETS_URL}…");
    match download_and_unpack_assets(&data_dir) {
        Ok(()) => println!("assets installed in {}", data_dir.display()),
        Err(e) => eprintln!("asset download failed: {e}"),
    }
}

/// Print which graphics backend eframe actually created, and on what GPU.
///
/// eframe has no backend-name API: `CreationContext` carries one handle per
/// compiled backend (`gl` for glow, `wgpu_render_state` behind the `wgpu`
/// feature) and the *presence* of a handle is the portable signal — so this
/// matches on the handles rather than assuming a backend. Each arm then
/// uses that backend's own introspection: wgpu's `AdapterInfo` names the
/// API and GPU directly; glow's cached [`eframe::glow::Version`] (a safe
/// call) distinguishes OpenGL from OpenGL ES, with only the GPU-name
/// string needing a raw `glGetString`.
pub(crate) fn log_renderer_info(cc: &eframe::CreationContext<'_>) {
    #[cfg(feature = "wgpu")]
    if let Some(render_state) = cc.wgpu_render_state.as_ref() {
        let info = render_state.adapter.get_info();
        println!("Renderer: {:?} on {} ({:?}).", info.backend, info.name, info.device_type);
        return;
    }
    if let Some(gl) = cc.gl.as_ref() {
        use eframe::glow::HasContext as _;
        let api = if gl.version().is_embedded { "OpenGL ES" } else { "OpenGL" };
        // Safety: eframe made this context current on this thread for the
        // duration of the creation closure, and VERSION/RENDERER are valid
        // `glGetString` enums.
        let (version, renderer) = unsafe {
            (
                gl.get_parameter_string(eframe::glow::VERSION),
                gl.get_parameter_string(eframe::glow::RENDERER),
            )
        };
        println!("{} version: {}, renderer: {}.", api, version, renderer);
        return;
    }
    println!("Renderer: unknown backend.");
}

fn main() -> eframe::Result<()> {
    setup_logging();
    banner();
    ensure_assets();

    // Bare `coco` (no CLI arguments) opens the CocoVM manager window; any
    // argument keeps the direct-boot emulator path below.
    if std::env::args_os().len() == 1 {
        return manager::run();
    }

    let cli = Cli::parse();
    let variant = cli.machine;
    let memory = cli.ram.unwrap_or_else(|| default_ram(variant));
    let config = MachineConfig {
        variant,
        video: cli.video,
        memory,
        // An explicit --monitor on a CoCo 1/2 flows through as Some so
        // `validate` below rejects it with the real reason (no monitor
        // port) instead of silently ignoring the flag.
        monitor: match variant {
            MachineVariant::Coco3 => {
                Some(cli.monitor.map_or(MonitorType::RGB, Into::into))
            }
            MachineVariant::Coco1 | MachineVariant::Coco2 => cli.monitor.map(Into::into),
        },
        // No CLI flag for this yet; same family default as the "New…"
        // dialog and the manager's detail pane (`default_vdg`).
        vdg: default_vdg(variant),
    };
    if let Err(e) = config.validate() {
        eprintln!("coco: invalid configuration: {e}");
        std::process::exit(1);
    }
    let roms_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    let rom = match cli.rom {
        Some(path) => load_explicit_rom(&path),
        None => load_default_rom(variant, &roms_dir),
    };
    let rom = match rom {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("coco: {e}");
            eprintln!("Pass --rom <PATH> to boot a specific image.");
            std::process::exit(1);
        }
    };
    let mpi = cli.mpi;
    let cart_path = cli.cart;
    let disk_paths = [cli.disk0, cli.disk1];
    let vhd_paths = [cli.vhd0, cli.vhd1];
    let fd502 = cli.fd502;
    let rtc = cli.rtc;
    let save_tape_wav = cli.tape_wav;
    let print_capture = cli.print_capture;
    // Without --mpi, --cart, --disk0/--disk1/--fd502, and --rtc all want the
    // single cartridge port (clap's declarative `conflicts_with` can't
    // express "only when --mpi is absent", so this is checked by hand).
    let port_claims = [
        cart_path.is_some(),
        disk_paths[0].is_some() || disk_paths[1].is_some() || fd502,
        rtc,
    ]
    .into_iter()
    .filter(|&claims| claims)
    .count();
    if !mpi && port_claims > 1 {
        eprintln!(
            "coco: --cart, --disk0/--disk1/--fd502, and --rtc all need the cartridge port; \
             combine them only with --mpi"
        );
        std::process::exit(1);
    }
    // Size for the aspect-corrected (wider) image so it always fits; the
    // uncorrected image is narrower and simply leaves margin.
    let img_h = coco_core::video::FB_H as f32 * SCALE;
    let win_w = img_h * TARGET_ASPECT;
    let win_h = img_h + MENU_BAR_H + TOOLBAR_H + STATUS_BAR_H;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    let window_title = format!("coco-rs — {}", machine_label(variant));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([win_w, win_h])
            .with_icon(icon)
            .with_title(&window_title),
        ..Default::default()
    };
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(move |cc| {
            log_renderer_info(cc);
            // With --mpi, --cart/--disk0/--disk1/--fd502 target MPI slots instead of
            // the plain single-cartridge model, so the base constructor gets none of
            // them and everything is wired up afterward through the same methods the
            // MultiPak menu uses.
            let mut app = if mpi {
                CocoApp::new(config, rom, None, [None, None], vhd_paths, save_tape_wav)
            } else {
                CocoApp::new(config, rom, cart_path.clone(), disk_paths.clone(), vhd_paths, save_tape_wav)
            };
            if mpi {
                app.insert_multipak();
                if let Some(path) = cart_path {
                    app.mpi_insert_rompak(0, path);
                }
                if fd502 || disk_paths[0].is_some() || disk_paths[1].is_some() {
                    app.mpi_insert_fd502(MPI_SLOT_COUNT - 1);
                }
                if rtc {
                    app.mpi_insert_rtc(DEFAULT_RTC_SLOT);
                }
                for (drive, path) in disk_paths.into_iter().enumerate() {
                    if let Some(path) = path {
                        app.insert_disk(drive, path);
                    }
                }
            } else if fd502 && let Err(e) = app.ensure_disk_controller() {
                app.cart_error = Some(e);
            } else if rtc {
                app.insert_rtc();
            }
            if let Some(path) = print_capture {
                app.start_print_capture(path);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parse_machine_accepts_known_spellings_and_rejects_others() {
        assert_eq!(parse_machine("coco1"), Ok(MachineVariant::Coco1));
        assert_eq!(parse_machine("coco2"), Ok(MachineVariant::Coco2));
        assert_eq!(parse_machine("coco3"), Ok(MachineVariant::Coco3));
        assert!(parse_machine("coco4").is_err());
        assert!(parse_machine("").is_err());
    }

    #[test]
    fn parse_ram_accepts_every_memory_size_spelling() {
        assert_eq!(parse_ram("4k"), Ok(MemorySize::K4));
        assert_eq!(parse_ram("16k"), Ok(MemorySize::K16));
        assert_eq!(parse_ram("32k"), Ok(MemorySize::K32));
        assert_eq!(parse_ram("64k"), Ok(MemorySize::K64));
        assert_eq!(parse_ram("128k"), Ok(MemorySize::K128));
        assert_eq!(parse_ram("512k"), Ok(MemorySize::K512));
        assert_eq!(parse_ram("2048k"), Ok(MemorySize::K2048));
        assert!(parse_ram("1mb").is_err());
    }

    #[test]
    fn parse_video_accepts_ntsc_and_pal() {
        assert_eq!(parse_video("ntsc"), Ok(VideoStandard::NTSC));
        assert_eq!(parse_video("pal"), Ok(VideoStandard::PAL));
        assert!(parse_video("secam").is_err());
    }

    #[test]
    fn default_ram_is_512k_for_coco3_and_64k_for_coco1_2() {
        assert_eq!(default_ram(MachineVariant::Coco3), MemorySize::K512);
        assert_eq!(default_ram(MachineVariant::Coco1), MemorySize::K64);
        assert_eq!(default_ram(MachineVariant::Coco2), MemorySize::K64);
    }

    #[test]
    fn default_vdg_is_t1_for_coco2_and_plain_elsewhere() {
        assert_eq!(default_vdg(MachineVariant::Coco2), Some(VDGVariant::MC6847T1));
        assert_eq!(default_vdg(MachineVariant::Coco1), Some(VDGVariant::MC6847));
        assert_eq!(default_vdg(MachineVariant::Coco3), None);
    }

    /// Scratch directory under `target/` holding only the ROM files a given
    /// test writes into it — deliberately not the real workspace `roms/`
    /// (whose contents vary machine-to-machine), so [`compose_coco12_rom`]'s
    /// candidate-preference logic is exercised deterministically.
    fn scratch_roms_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp-test-roms")
            .join(name);
        std::fs::create_dir_all(&dir).expect("create scratch roms dir");
        dir
    }

    #[test]
    fn compose_coco12_rom_lays_out_extbas_then_bas() {
        let dir = scratch_roms_dir("compose_with_extbas");
        let bas = vec![0xAAu8; COCO12_BAS_OFFSET];
        let extbas = vec![0xBBu8; COCO12_BAS_OFFSET];
        std::fs::write(dir.join("bas12.rom"), &bas).unwrap();
        std::fs::write(dir.join("extbas11.rom"), &extbas).unwrap();

        let Coco12RomResult::Composed { image, .. } = compose_coco12_rom(&dir) else {
            panic!("expected Composed");
        };
        assert_eq!(image.len(), COCO12_BAS_OFFSET * 2);
        assert_eq!(&image[..COCO12_BAS_OFFSET], &extbas[..]);
        assert_eq!(&image[COCO12_BAS_OFFSET..], &bas[..]);
    }

    #[test]
    fn compose_coco12_rom_fills_open_bus_when_extbas_missing() {
        let dir = scratch_roms_dir("compose_without_extbas");
        let bas = vec![0xAAu8; COCO12_BAS_OFFSET];
        std::fs::write(dir.join("bas12.rom"), &bas).unwrap();

        let Coco12RomResult::Composed { image, extbas, .. } = compose_coco12_rom(&dir) else {
            panic!("expected Composed");
        };
        assert!(extbas.is_none());
        assert!(
            image[..COCO12_BAS_OFFSET]
                .iter()
                .all(|&b| b == OPEN_BUS_FILLER)
        );
        assert_eq!(&image[COCO12_BAS_OFFSET..], &bas[..]);
    }

    #[test]
    fn compose_coco12_rom_prefers_newest_candidate_present() {
        let dir = scratch_roms_dir("compose_prefers_newest");
        // bas10 and bas12 both present: bas12 (newer) must win.
        std::fs::write(dir.join("bas10.rom"), vec![0x10u8; COCO12_BAS_OFFSET]).unwrap();
        std::fs::write(dir.join("bas12.rom"), vec![0x12u8; COCO12_BAS_OFFSET]).unwrap();

        let Coco12RomResult::Composed { bas, .. } = compose_coco12_rom(&dir) else {
            panic!("expected Composed");
        };
        assert_eq!(bas.0.file_name().unwrap(), "bas12.rom");
    }

    #[test]
    fn unpack_assets_extracts_gzipped_tar_into_dest() {
        let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp-test-assets/unpack");
        let _ = std::fs::remove_dir_all(&dest);

        // Build a cocovm-assets-shaped tarball in memory: roms/ and images/.
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tarball = tar::Builder::new(gz);
        for (path, contents) in [
            ("roms/test.rom", &b"\xAA\xBB"[..]),
            ("images/blank.dsk", &b"\x00\x01"[..]),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tarball.append_data(&mut header, path, contents).unwrap();
        }
        let bytes = tarball.into_inner().unwrap().finish().unwrap();

        unpack_assets(&bytes[..], &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("roms/test.rom")).unwrap(), b"\xAA\xBB");
        assert_eq!(std::fs::read(dest.join("images/blank.dsk")).unwrap(), b"\x00\x01");
    }

    #[test]
    fn compose_coco12_rom_demotes_coco2b_bas13_to_last_resort() {
        let dir = scratch_roms_dir("compose_demotes_bas13");
        // bas13 pairs with the unmodeled MC6847T1 (CoCo 2B): bas12 must win
        // over it despite being the older version number.
        std::fs::write(dir.join("bas13.rom"), vec![0x13u8; COCO12_BAS_OFFSET]).unwrap();
        std::fs::write(dir.join("bas12.rom"), vec![0x12u8; COCO12_BAS_OFFSET]).unwrap();

        let Coco12RomResult::Composed { bas, .. } = compose_coco12_rom(&dir) else {
            panic!("expected Composed");
        };
        assert_eq!(bas.0.file_name().unwrap(), "bas12.rom");
    }

    #[test]
    fn compose_coco12_rom_reports_missing_color_basic() {
        let dir = scratch_roms_dir("compose_no_bas");
        // Directory exists but has no candidate ROMs in it.
        assert!(matches!(
            compose_coco12_rom(&dir),
            Coco12RomResult::NoColorBasic
        ));
    }
}

#[cfg(test)]
mod ui_tests;
