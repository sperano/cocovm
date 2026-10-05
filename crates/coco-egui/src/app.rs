//! The emulator app itself: the machine and all the UI state hanging off it.
//! Host input lives in `app::input` and the per-frame loop in `app::frame`;
//! the chrome around the display is `chrome`, and everything mountable in the
//! machine is `media`.

use crate::*;

mod control;
pub(crate) use control::{PAUSED_ERROR, RemoteHold, RemoteStick};
mod frame;
#[cfg(test)]
pub(crate) use frame::SUSPENDED_SCRIM;
mod input;
mod presentation;
pub(crate) mod scheduling;
pub(crate) use input::has_keyboard_focus;

pub(crate) struct CocoApp {
    pub(crate) machine: Machine,
    pub(crate) texture: Option<egui::TextureHandle>,
    pub(crate) running: bool,
    pub(crate) kb_mode: KbMode,
    pub(crate) type_ahead: TypeAhead,
    pub(crate) show_kbd_help: bool,
    pub(crate) keyboard_modifiers: typeahead::KeyModifiers,
    pub(crate) show_about: bool,
    /// "View > Orchestra-90 Levels" window toggle ([`orch90_meters::window`]).
    /// Stays whatever the user last set even if the cartridge is later
    /// ejected — the window doesn't draw without a live `Orch90`
    /// (see the call site in `update`).
    pub(crate) show_orch90: bool,
    /// What the video output is plugged into — monitor or (B&W) TV
    /// (`display.rs`). A UI preference seeded from
    /// the config here, overridden by the definition's `[hardware].display`
    /// (`launch::launch_machine_with_gamepad`), and live-switchable from the status bar's
    /// display entry afterwards.
    pub(crate) display: Display,
    /// The TV chain's knobs (scanline strength, …) — same lifecycle as
    /// `display`: `[ui]` keys provide the starting values, and the display
    /// menu's sliders update them. Only consulted while `display` is a TV.
    pub(crate) tv: display::TVSettings,
    /// Retained display buffers and the last presented pixel/settings state.
    pub(crate) presentation: presentation::Presentation,
    pub(crate) schedule: scheduling::Schedule,
    /// Wall-clock instant of the previous update while running; `None` right
    /// after a pause/start so the first frame credits no elapsed time.
    pub(crate) last_update: Option<std::time::Instant>,
    /// Fractional emulated fields owed to the wall clock (`DESIGN.md` §4):
    /// fields run when it reaches 1, the remainder carries over. This decouples
    /// emulation speed from the host refresh rate (120 Hz displays no longer
    /// run the CoCo at double speed).
    pub(crate) field_debt: f64,
    /// Fields run ahead of the wall clock on entering the background throttle,
    /// so the audio ring holds a cushion across each gap; owed back on return.
    pub(crate) audio_cushion_fields: usize,
    /// Per-port joystick source selection (mouse/gamepad/keys) and gamepad state.
    pub(crate) joysticks: JoystickInputs,
    /// cpal output stream, resampler, and volume/mute state (`audio.rs`).
    pub(crate) audio: audio::AudioOutput,
    /// Letterboxed display rect from the last frame's `CentralPanel`, used to map
    /// pointer position to joystick axes and to gate the mouse fire buttons to
    /// presses starting on the display. One frame stale (see `drive_joysticks`).
    pub(crate) display_rect: egui::Rect,
    /// Layer `draw_display` drew the display on last frame: the background
    /// layer for a full native window's `CentralPanel`, or the `egui::Window`'s
    /// own layer in the manager's embedded fallback. Paired with `display_rect`
    /// to gate the mouse fire buttons — a press whose topmost egui layer is
    /// neither this nor bare panel landed on a popup/window floating over the
    /// display (see `joy::press_began_on_display`).
    pub(crate) display_layer: egui::LayerId,
    /// Path of the currently inserted cartridge, if any (shown in the status
    /// bar).
    pub(crate) cart_path: Option<PathBuf>,
    /// Message from the last failed cartridge load, shown in a dismissible
    /// window until acknowledged.
    pub(crate) cart_error: Option<String>,
    /// Source paths of the floppies mounted in the FD-502's drives the UI
    /// exposes (status bar, eject menu items, and write-back targets — a
    /// modified image is written back to its file on eject/replace/exit).
    pub(crate) disk_paths: [Option<PathBuf>; UI_DRIVES],
    /// ROM source for the single FD-502, in the direct port or an MPI slot.
    pub(crate) disk_rom_path: Option<PathBuf>,
    /// Source paths of the VHD (virtual hard disk) images mounted in the two
    /// drives the UI exposes (status bar). Unlike `disk_paths`, VHD writes
    /// hit the backing file directly — there is no in-memory dirty state
    /// and so nothing to write back on exit.
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
    /// The tape menu's "Seek to byte" field ([`CocoApp::tape_menu_ui`]),
    /// edited live and committed to [`coco_core::cassette::Cassette::seek`]
    /// on Enter — not live-rebound on each keystroke, like `rs232_tcp_addr`.
    pub(crate) tape_seek_text: String,
    /// Destination path of the active bit-banger "print to text file"
    /// capture, if any — shown in the Machine
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
    /// State of the inserted Multi-Pak Interface, if any — `None` means the
    /// cartridge slot holds a plain cartridge (or nothing), today's default.
    pub(crate) mpi: Option<MPIState>,
    /// State of the inserted Deluxe RS-232 Program Pak, if any: which host
    /// endpoint its serial line is wired to (the core's trait object can't
    /// describe itself to status-bar labels, so the frontend tracks it —
    /// same rationale as [`MPISlot`]). `None` means the slot holds something
    /// else.
    pub(crate) rs232: Option<RS232Endpoint>,
    /// Source path of the Deluxe RS-232 pak's optional EPROM dump, if one was
    /// found and installed at insert time ([`Self::insert_rs232`]) — the
    /// save-state counterpart of `cart_path` for this one cart, since the
    /// pak can legitimately run ROM-less.
    pub(crate) rs232_eprom_path: Option<PathBuf>,
    /// Listen address for the RS-232 pak's TCP endpoint, taken from
    /// `[peripherals].cartridge.endpoint.listen` at launch
    /// (`launch::mount_rs232`) and reused verbatim when a Load State
    /// reapplies `rs232_configured`.
    pub(crate) rs232_tcp_addr: String,
    /// Which non-default endpoint kind `[peripherals]` configured the RS-232
    /// pak with at launch, if any — `None` for loopback (the pak's own
    /// restored default, so nothing needs reapplying). Load State drops the
    /// core's live endpoint (`#[serde(skip)]`,
    /// [`coco_core::rs232::DeluxeRS232::endpoint`]'s doc) and restores
    /// loopback; [`Self::rebuild_cart_mirrors`] rebinds this kind
    /// afterward through [`Self::rs232_set_endpoint`] so a configured TCP/PTY
    /// endpoint survives the round trip.
    pub(crate) rs232_configured: Option<RS232EndpointKind>,
    /// True while a Disto RTC is plugged directly into the cartridge port,
    /// like `cart_path` tracks a ROM Pak. An RTC in a Multi-Pak slot is
    /// tracked by [`MPISlot::DistoRTC`] instead.
    pub(crate) rtc_direct: bool,
    /// The virtual fanfold-paper window, showing
    /// the DMP printer's dot-matrix output on period-correct tractor-feed
    /// stationery. See [`Self::toggle_paper_window`] for the sink-ownership
    /// handshake with print-file-capture.
    pub(crate) paper_window: paper_view::PaperWindow,
    /// The interactive debugger: breakpoints,
    /// watchpoints, and the Controls/Registers/Disassembly/Memory/Stack/
    /// Hardware panel cluster, toggled with the toolbar's Debug tile or ⌘D.
    pub(crate) debugger: debugger::DebuggerPanel,
    /// Where the currently-loaded system ROM image came from, for
    /// [`Self::save_state_to`] (`save_state.rs`) to record and re-resolve —
    /// see [`ROMSource`].
    pub(crate) rom_source: ROMSource,
    /// Status-bar toast: a message plus when it was shown
    /// ([`Self::set_toast`], `save_state.rs`), displayed for
    /// [`save_state::TOAST_SECS`] seconds. Save/load-state results use it now,
    /// and other fire-and-forget confirmations can use it later.
    pub(crate) toast: Option<(String, std::time::Instant)>,
    /// Status-bar device-activity icons: per-device pulse-stretched latches
    /// over the core's monotonic activity counters, plus the cassette reel
    /// angle (`status_icons.rs`). Purely UI state — not serialized, not
    /// touched by save/load state.
    pub(crate) activity: StatusActivity,
    /// Set by the VM window's Suspend tile (`chrome::toolbar`); consumed by
    /// [`crate::manager::ManagerApp::draw_running_vms`] after the viewport
    /// closure returns. This field is only a *request* — `CocoApp` cannot suspend itself
    /// (it doesn't even know its own slug); the actual freeze (screenshot →
    /// `.ccstate` → pause) is manager-owned
    /// (`manager::lifecycle::suspend_vm`).
    pub(crate) pending_suspend: bool,
    /// [`Self::pending_suspend`]'s twin for the Start tile of a suspended
    /// window: a request the manager turns into
    /// `manager::lifecycle::resume_vm`.
    pub(crate) pending_resume: bool,
    /// Mirror of the manager's Suspended flag for this VM, set every frame
    /// before the window draws (`manager::vm_windows`). While set the chrome
    /// is a read-only view: nothing may change the machine, which would
    /// silently diverge it from its frozen `.ccstate`.
    pub(crate) suspended: bool,
    /// Whether the last frame drew this window suspended — detects the
    /// Running -> Suspended edge so `window_ui` can close any menu
    /// or status popup still open from before (`ui.disable()` doesn't reach
    /// an already-open popup).
    pub(crate) drew_suspended: bool,
    /// Cumulative powered-on time across all of this machine's sessions,
    /// including the one running right now. Accumulated in
    /// [`Self::fields_due`] (`app/frame.rs`) from the same `MAX_FRAME_DT`-
    /// clamped `dt` that feeds `field_debt`, so a host stall is credited at
    /// most that cap rather than its full unclamped length. Naturally stops
    /// advancing whenever `last_update` is `None` (paused/suspended — see
    /// that field's doc) — `fields_due` is only ever called from the
    /// `running` branch of `step_emulation`, so a paused interval (debugger
    /// breakpoint included) never reaches this field at all. Seeded at
    /// launch from the persisted `[stats].runtime_secs`
    /// (`launch::launch_machine_with_gamepad`), and persisted by
    /// [`manager::lifecycle::fold_runtime_into_def`] on Suspend, Stop, and
    /// quit.
    pub(crate) total_runtime: std::time::Duration,
    /// Text queued by a control-protocol `type_text` request — a second,
    /// independent [`TypeAhead`] instance so remote typing never fights the
    /// host's own `type_ahead` for the same queue (`app/frame.rs::run_fields`
    /// advances both). Driven regardless of host keyboard focus — see
    /// `app/input.rs`'s `release_keyboard_state` doc.
    pub(crate) remote_type_ahead: TypeAhead,
    /// A control-protocol `press_keys` hold in progress: the positions held
    /// down and the fields remaining before [`Self::run_fields`]
    /// (`app/frame.rs`) releases them. `None` when no hold is active — only
    /// one may be in flight at a time.
    pub(crate) remote_held: Option<RemoteHold>,
    /// Per-port joystick override from a control-protocol `joystick`
    /// request, indexed like [`JoystickInputs::sources`]
    /// (`coco_core::joystick::{LEFT, RIGHT}`). Applied in
    /// [`Self::drive_joysticks`] (`app/input.rs`) after the host's own
    /// [`JoystickInputs::apply`], so it wins over an unassigned port's
    /// recenter. `None` hands the port back to the host source.
    pub(crate) remote_joy: [Option<RemoteStick>; 2],
    /// Fields actually completed by [`Self::run_fields`] (`app/frame.rs`),
    /// monotonic for this VM's lifetime — the control protocol's `wait`
    /// request polls this to know when its target field count has elapsed.
    pub(crate) fields_run: u64,
    /// Mirror of [`crate::manager::ManagerApp`]'s global `toolbar_icons_only`
    /// (`config.rs`), read by `chrome::toolbar`; the manager rewrites it every
    /// frame (`manager/vm_windows.rs`), so it is not an [`AppParams`] field.
    pub(crate) toolbar_icons_only: bool,
    /// `toolbar_icons_only`'s counterpart for the status bar
    /// (`chrome::status_bar`): entries draw their icon alone, with the
    /// readout moved into hover text. Rewritten every frame the same way.
    pub(crate) status_bar_icons_only: bool,
    /// The manager's rebindable hotkeys (`hotkeys.rs`), consumed in
    /// `app/input.rs` and named in the menus and help. Rewritten every
    /// frame the same way.
    pub(crate) hotkeys: crate::hotkeys::Hotkeys,
}

/// DriveWire launch settings — the payload of [`AppParams::drivewire`],
/// whose `Some`/`None` is what enables/disables the Becker port at boot.
#[derive(Default)]
pub(crate) struct DriveWireLaunch {
    /// Serve HDB-DOS sector addressing instead of plain DriveWire —
    /// forwarded to `DWServer::set_hdbdos_mode`.
    pub(crate) hdbdos_mode: bool,
    /// Images to mount, one slot per DriveWire drive.
    pub(crate) disk_paths: [Option<PathBuf>; drivewire::DRIVE_COUNT],
}

/// Everything [`CocoApp::new`] mounts or configures beyond the machine
/// itself. `Default` is a bare machine: no media, DriveWire off, tape-wav
/// capture off — what every test call site wants outright, and what the
/// production launcher (`launch::new_app`) wants for everything but the
/// media it mounts.
#[derive(Default)]
pub(crate) struct AppParams {
    pub(crate) cart_path: Option<PathBuf>,
    pub(crate) vhd_paths: [Option<PathBuf>; UI_DRIVES],
    /// `Some` boots with the Becker port enabled ([`DriveWireLaunch`]).
    /// The production launcher builds this from `[drivewire]`; guest-selected
    /// runtime mounts remain independent of these startup assignments.
    pub(crate) drivewire: Option<DriveWireLaunch>,
    /// UI preference, not persisted per-machine yet — always `false` at
    /// launch, toggled at runtime in the tape menu (status bar's Cassette
    /// deck entry, `chrome/menu_bar.rs`'s `tape_menu_ui`).
    pub(crate) save_tape_wav: bool,
}

impl CocoApp {
    /// No `CreationContext` param, unlike most `eframe::App` constructors:
    /// nothing here touches egui context state, so this is a plain constructor.
    pub(crate) fn new(
        config: MachineConfig,
        rom: Box<[u8]>,
        rom_source: ROMSource,
        params: AppParams,
        gamepad: crate::joy::SharedGamepad,
    ) -> Self {
        let AppParams {
            cart_path,
            vhd_paths,
            drivewire,
            save_tape_wav,
        } = params;
        // Lossy for a CoCo 3 TV; launch/boot overwrite it with the real choice afterwards.
        let display = Display::from_config(&config);
        let mut app = Self {
            machine: Machine::new(config, rom),
            texture: None,
            running: true, // boot straight to the prompt
            kb_mode: KbMode::Positional,
            type_ahead: TypeAhead::default(),
            show_kbd_help: false,
            keyboard_modifiers: typeahead::KeyModifiers::default(),
            show_about: false,
            show_orch90: false,
            display,
            tv: display::TVSettings::default(),
            presentation: presentation::Presentation::default(),
            schedule: scheduling::Schedule::default(),
            last_update: None,
            field_debt: 0.0,
            audio_cushion_fields: 0,
            total_runtime: std::time::Duration::ZERO,
            joysticks: JoystickInputs::new(gamepad),
            display_rect: egui::Rect::NOTHING,
            display_layer: egui::LayerId::background(),
            audio: audio::AudioOutput::new(),
            cart_path: None,
            disk_rom_path: None,
            cart_error: None,
            disk_paths: [None, None],
            vhd_paths: [None, None],
            dw_paths: std::array::from_fn(|_| None),
            tape_path: None,
            save_tape_wav,
            tape_seek_text: String::new(),
            print_capture_path: None,
            print_capture_lf: false,
            mpi: None,
            rs232: None,
            rs232_eprom_path: None,
            rs232_tcp_addr: RS232_TCP_DEFAULT_ADDR.to_string(),
            rs232_configured: None,
            rtc_direct: false,
            paper_window: paper_view::PaperWindow::new(),
            debugger: debugger::DebuggerPanel::new(),
            rom_source,
            toast: None,
            activity: StatusActivity::default(),
            pending_suspend: false,
            pending_resume: false,
            suspended: false,
            drew_suspended: false,
            remote_type_ahead: TypeAhead::default(),
            remote_held: None,
            remote_joy: [None, None],
            fields_run: 0,
            toolbar_icons_only: false,
            status_bar_icons_only: false,
            hotkeys: crate::hotkeys::Hotkeys::default(),
        };
        if let Some(path) = cart_path {
            app.insert_cartridge(path);
        }
        for (drive, path) in vhd_paths.into_iter().enumerate() {
            if let Some(path) = path {
                app.insert_vhd(drive, path);
            }
        }
        if let Some(dw) = drivewire {
            app.enable_drivewire(dw.hdbdos_mode);
            for (drive, path) in dw.disk_paths.into_iter().enumerate() {
                if let Some(path) = path {
                    app.insert_dw_disk(drive, path);
                }
            }
        }
        app
    }

    /// Write modified floppies and tape back to their files. Tries both even if
    /// one fails — independent media — and joins error messages with `\n`.
    pub(crate) fn flush_media(&mut self) -> Result<(), String> {
        let disks = self.flush_dirty_disks();
        let tape = self.write_back_tape();
        match (disks, tape) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
            (Err(d), Err(t)) => Err(format!("{d}\n{t}")),
        }
    }

    /// Poll completed DriveWire host work without blocking the UI thread.
    pub(crate) fn poll_drivewire_host(&mut self) {
        if let Some(dw) = self.machine.bus.drivewire.as_mut() {
            dw.poll_host();
        }
    }

    /// Stop DriveWire host work before this VM is dropped or replaced.
    pub(crate) fn stop_drivewire_host(&mut self) {
        if let Some(dw) = self.machine.bus.drivewire.as_mut() {
            dw.stop_host();
        }
    }

    /// Suspend DriveWire host work after the VM reaches an idle save point.
    pub(crate) fn suspend_drivewire_host(&mut self) {
        if let Some(dw) = self.machine.bus.drivewire.as_mut() {
            dw.suspend_host();
        }
    }

    /// Resume DriveWire host work before emulation starts again.
    pub(crate) fn resume_drivewire_host(&mut self) {
        if let Some(dw) = self.machine.bus.drivewire.as_mut() {
            dw.resume_host();
        }
    }

    /// Set whether emulation advances. Exposed since `running` isn't `pub`;
    /// used by the manager's Suspend/Resume to freeze/un-freeze a VM.
    pub(crate) fn set_running(&mut self, running: bool) {
        if self.running != running {
            self.reset_emulation_clock();
        }
        self.running = running;
    }

    /// Whether emulation is currently advancing. Test-only — production code
    /// only ever sets `running`, never reads it back.
    #[cfg(test)]
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    /// The framebuffer texture reused until pixels or display settings change;
    /// `None` only before the VM's first frame. The manager uses it to
    /// reuse the same handle to draw a list-row thumbnail.
    pub(crate) fn framebuffer_texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }
}

/// Test scaffolding only: production code never runs a `CocoApp` through
/// `eframe::run_native` (the manager calls [`CocoApp::window_ui`] and
/// [`CocoApp::flush_media`] directly on VMs it owns), but `ui_tests::harness`'s
/// `boot_harness` still builds a plain `egui_kittest::Harness<CocoApp>`, which
/// needs this impl to exist.
impl eframe::App for CocoApp {
    /// Write modified floppies and tape back to their files on quit. Failures
    /// are only logged — the app is going away, so there's no dialog to show them in.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_drivewire_host();
        if let Err(e) = self.flush_media() {
            tracing::warn!("could not flush media on exit: {e}");
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let delay = scheduling::background_delay(ctx);
        self.window_ui(ctx, delay);
    }
}

#[cfg(test)]
#[path = "app_test.rs"]
mod tests;
