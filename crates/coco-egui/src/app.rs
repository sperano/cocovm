//! The emulator app itself: the machine and all the UI state hanging off it.
//! Host input lives in `app::input` and the per-frame loop in `app::frame`;
//! the chrome around the display is `chrome`, and everything mountable in the
//! machine is `media`.

use crate::*;

mod frame;
mod input;

pub(crate) struct CocoApp {
    pub(crate) machine: Machine,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) running: bool,
    pub(crate) kb_mode: KbMode,
    pub(crate) type_ahead: TypeAhead,
    pub(crate) show_kbd_help: bool,
    pub(crate) show_about: bool,
    /// "View > Orchestra-90 Levels" window toggle ([`orch90_meters::window`]).
    /// Stays whatever the user last set even if the cartridge is later
    /// ejected — the window simply doesn't draw without a live `Orch90`
    /// (see the call site in `update`).
    pub(crate) show_orch90: bool,
    pub(crate) aspect_correct: bool,
    /// Wall-clock instant of the previous update while running; `None` right
    /// after a pause/start so the first frame credits no elapsed time.
    pub(crate) last_update: Option<std::time::Instant>,
    /// Fractional emulated fields owed to the wall clock (`DESIGN.md` §4):
    /// fields run when it reaches 1, the remainder carries over. This decouples
    /// emulation speed from the host refresh rate (120 Hz displays no longer
    /// run the CoCo at double speed).
    pub(crate) field_debt: f64,
    /// Per-port joystick source selection (mouse/gamepad/keys) and gamepad state.
    pub(crate) joysticks: JoystickInputs,
    /// cpal output stream, resampler, and volume/mute state (`audio.rs`).
    pub(crate) audio: audio::AudioOutput,
    /// Letterboxed display rect from the last frame's `CentralPanel`, used to map
    /// pointer position to joystick axes. One frame stale (see `drive_joysticks`).
    pub(crate) display_rect: egui::Rect,
    /// Whether the next inserted cartridge should tie CART* to Q (auto-run at
    /// power-up). Consulted at insert time, not retroactively — see
    /// `RomPak::from_bytes`. Off suits Disk-BASIC-style paks and carts that
    /// must be started with `EXEC &HE010`.
    pub(crate) autostart_cart: bool,
    /// Path of the currently inserted cartridge, if any (shown in the status
    /// bar; also gates the "Eject Cartridge" menu item).
    pub(crate) cart_path: Option<PathBuf>,
    /// Message from the last failed cartridge load, shown in a dismissible
    /// window until acknowledged.
    pub(crate) cart_error: Option<String>,
    /// Source paths of the floppies mounted in the FD-502's drives the UI
    /// exposes (status bar, eject menu items, and write-back targets — a
    /// modified image is written back to its file on eject/replace/exit).
    pub(crate) disk_paths: [Option<PathBuf>; UI_DRIVES],
    /// Source paths of the VHD (virtual hard disk) images mounted in the two
    /// drives the UI exposes (status bar, eject menu items). Unlike
    /// `disk_paths`, VHD writes hit the backing file directly — there is no
    /// in-memory dirty state and so nothing to write back on eject/exit.
    pub(crate) vhd_paths: [Option<PathBuf>; UI_DRIVES],
    /// Source paths of the DriveWire disk images mounted in the four drives
    /// the UI exposes (status bar, eject menu items). Like `vhd_paths`, writes
    /// hit the backing file directly.
    pub(crate) dw_paths: [Option<PathBuf>; drivewire::DRIVE_COUNT],
    /// Source path of the mounted cassette tape (.cas), if any — the
    /// write-back target for recordings, like `disk_paths` for floppies.
    pub(crate) tape_path: Option<PathBuf>,
    /// Whether [`Self::write_back_tape`] should, in addition to the always-
    /// written canonical `.cas`, also synthesize and write a `.wav` of the
    /// tape audio (`coco_core::cassette_wav::synthesize_wav`) alongside it.
    pub(crate) save_tape_wav: bool,
    /// Destination path of the active bit-banger "print to text file"
    /// capture, if any (`docs/printer-plan.md` T2) — shown in the Machine
    /// menu and gates "Stop Print Capture", like `tape_path` does for the
    /// cassette deck. Unlike disk/tape images, there is nothing to write
    /// back on eject: `coco_core::bitbanger::FileSink` writes straight
    /// through as bytes are decoded.
    pub(crate) print_capture_path: Option<PathBuf>,
    /// Machine-menu "Translate CR to LF" checkbox: when set, print captures
    /// rewrite the CoCo's bare-CR line endings as LF so the file reads as
    /// normal host text (faithful raw bytes otherwise). Applies when a
    /// capture starts — an in-progress capture keeps the mode it began with.
    pub(crate) print_capture_lf: bool,
    /// A disk action waiting on the "this will power-cycle the machine"
    /// confirmation dialog — set instead of acting when the FD-502 isn't in
    /// the cartridge slot yet, since inserting it swaps the cartridge and
    /// cold-restarts the machine (unsaved state is lost).
    pub(crate) pending_disk_action: Option<PendingDiskAction>,
    /// State of the inserted Multi-Pak Interface, if any — `None` means the
    /// cartridge slot holds a plain cartridge (or nothing), today's default.
    pub(crate) mpi: Option<MPIState>,
    /// State of the inserted Deluxe RS-232 Program Pak, if any: which host
    /// endpoint its serial line is wired to (the core's trait object can't
    /// describe itself to menu labels, so the frontend tracks it — same
    /// rationale as [`MPISlot`]). `None` means the slot holds something else.
    pub(crate) rs232: Option<Rs232Endpoint>,
    /// Source path of the Deluxe RS-232 pak's optional EPROM dump, if one was
    /// found and installed at insert time ([`Self::insert_rs232`]) — the
    /// save-state counterpart of `cart_path` for this one cart, since the
    /// pak can legitimately run ROM-less (`docs/plan-deluxe-rs232.md`).
    pub(crate) rs232_eprom_path: Option<PathBuf>,
    /// Listen address for the RS-232 pak's TCP endpoint, edited in the menu
    /// and applied when "TCP" is (re)selected — not live-rebound on each
    /// keystroke.
    pub(crate) rs232_tcp_addr: String,
    /// True while a Disto RTC is plugged directly into the cartridge port
    /// (gates the "Eject Disto RTC" menu item, like `cart_path` does for ROM
    /// paks). An RTC in a Multi-Pak slot is tracked by [`MPISlot::DistoRTC`]
    /// instead.
    pub(crate) rtc_direct: bool,
    /// The virtual fanfold-paper window (`docs/printer-plan.md` T5), showing
    /// the DMP-105's dot-matrix output on period-correct tractor-feed
    /// stationery. See [`Self::toggle_paper_window`] for the sink-ownership
    /// handshake with print-file-capture.
    pub(crate) paper_window: paper_view::PaperWindow,
    /// The interactive debugger (`docs/plan-debugger.md` §3): breakpoints,
    /// watchpoints, and the Controls/Registers/Disassembly/Memory/Stack/
    /// Hardware panel cluster, toggled with F11.
    pub(crate) debugger: debugger::DebuggerPanel,
    /// Where the currently-loaded system ROM image came from, for
    /// [`Self::save_state_to`] (`save_state.rs`) to record and re-resolve —
    /// see [`RomSource`].
    pub(crate) rom_source: RomSource,
    /// Status-bar toast: a message plus when it was shown
    /// ([`Self::set_toast`], `save_state.rs`), displayed for
    /// [`save_state::TOAST_SECS`] seconds — save/load-state results today,
    /// extensible to any other fire-and-forget confirmation later.
    pub(crate) toast: Option<(String, std::time::Instant)>,
    /// Status-bar device-activity icons: per-device pulse-stretched latches
    /// over the core's monotonic activity counters, plus the cassette reel
    /// angle (`status_icons.rs`). Purely UI state — not serialized, not
    /// touched by save/load state.
    pub(crate) activity: StatusActivity,
}

/// See [`CocoApp::pending_disk_action`].
pub(crate) enum PendingDiskAction {
    Insert { drive: usize, path: PathBuf },
    NewBlank { drive: usize, path: PathBuf },
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: MachineConfig,
        rom: Box<[u8]>,
        rom_source: RomSource,
        cart_path: Option<PathBuf>,
        disk_paths: [Option<PathBuf>; UI_DRIVES],
        vhd_paths: [Option<PathBuf>; UI_DRIVES],
        dw_paths: [Option<PathBuf>; drivewire::DRIVE_COUNT],
        becker_enabled: bool,
        hdbdos_mode: bool,
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
            show_orch90: false,
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
            dw_paths: std::array::from_fn(|_| None),
            tape_path: None,
            save_tape_wav,
            print_capture_path: None,
            print_capture_lf: false,
            pending_disk_action: None,
            mpi: None,
            rs232: None,
            rs232_eprom_path: None,
            rs232_tcp_addr: RS232_TCP_DEFAULT_ADDR.to_string(),
            rtc_direct: false,
            paper_window: paper_view::PaperWindow::new(),
            debugger: debugger::DebuggerPanel::new(),
            rom_source,
            toast: None,
            activity: StatusActivity::default(),
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
        if becker_enabled {
            app.enable_drivewire(hdbdos_mode);
            for (drive, path) in dw_paths.into_iter().enumerate() {
                if let Some(path) = path {
                    app.insert_dw_disk(drive, path);
                }
            }
        } else {
            app.dw_paths = std::array::from_fn(|_| None);
        }
        app
    }

    /// Write modified floppies and tape back to their files — the exit
    /// contract [`eframe::App::on_exit`] runs for the direct-boot window,
    /// and the same one a manager-owned VM needs on Stop or on the
    /// manager's own `on_exit` (`ManagerApp`'s `eframe::App` impl in
    /// `manager.rs`, `docs/plan-machine-persistence.md` "one native window
    /// per running VM").
    pub(crate) fn flush_media(&mut self) {
        self.flush_dirty_disks();
        self.write_back_tape();
    }

    /// Set whether emulation advances — exposed so the manager's
    /// Suspend/Resume can freeze and un-freeze a VM it doesn't otherwise
    /// reach into (`running` has no `pub` visibility). The old user-facing
    /// Run/Pause toggle is gone (three-state model, user decision
    /// 2026-07-27); besides suspend, only the debugger still stops the
    /// clock, and it assigns `running` directly.
    pub(crate) fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Whether emulation is currently advancing (vs. frozen by Suspend or
    /// the debugger). Test-only since the user-facing Run/Pause chrome went
    /// away: production code drives `running` through [`Self::set_running`]
    /// and never needs to read it back.
    #[cfg(test)]
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
