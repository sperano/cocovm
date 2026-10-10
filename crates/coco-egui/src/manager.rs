//! The CoCoVM manager window: the VirtualBox/Parallels-style main window
//! `coco` always opens. Toolbar across the top, machine list down the left
//! (one row per `config_dir()/machines/<slug>.toml`, `machine_def.rs`), and
//! a detail/edit pane on the right for the selected machine — or, with no machine
//! selected, welcome instructions above a random photo (`manager/welcome.rs`).
//!
//! The detail pane's Start button calls `crate::launch_machine_with_gamepad`. Once a
//! `MachineEntry` holds a running `CocoApp`, `ManagerApp::update` opens it in
//! its own native OS window every frame—an *immediate viewport*, like the
//! printer-paper window in `paper_view::PaperWindow`. All VM state stays on
//! the main thread. Each viewport's child `egui::Context` delivers keyboard
//! and mouse input for that window, so egui handles focus routing
//! ("DECIDED: in-process, one native window per running VM"). The app always
//! opens this manager window; the CLI's optional slug starts a saved machine
//! from the manager's own definitions as it opens.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;

use crate::config::ManagerSort;
use crate::photo_view::Photo;
use crate::{CocoApp, machine_def};

use detail::EditState;
use selection::Selection;

pub(crate) mod assets;
mod bulk;
mod control;
mod delete;
mod detail;
mod detail_map;
mod exit;
mod gamepad_service;
mod lifecycle;
pub(crate) mod list;
mod live_drivewire;
#[cfg(feature = "perf")]
mod perf_scenarios;
mod rename;
pub(crate) mod reveal;
mod roms;
mod run;
mod selection;
mod settings;
mod sort;
mod thumbnails;
mod toolbar;
mod vm_windows;
mod welcome;
mod welcome_image;

pub use run::run;

/// Manager window size at first open.
const WINDOW_SIZE: [f32; 2] = [1080.0, 720.0];

/// Machine-list panel: width at first open and the draggable divider's range.
const LIST_DEFAULT_WIDTH: f32 = 260.0;
const LIST_MIN_WIDTH: f32 = 160.0;
const LIST_MAX_WIDTH: f32 = 520.0;

/// Corner rounding of the row thumbnail placeholder itself — distinct from
/// [`ROW_CORNER_RADIUS`], the row's own selection/hover frame.
const THUMBNAIL_CORNER_RADIUS: f32 = 2.0;
/// Fill of the row thumbnail placeholder — a powered-off machine's whole
/// preview (black, like the screen of a machine with no power — the state
/// model's own rule), a suspended one's backdrop until its saved
/// [`THUMBNAIL_FILE`] loads, and a running one's letterbox background
/// before its texture is uploaded/when the texture's aspect doesn't exactly
/// fill the allocated rect.
const THUMBNAIL_PLACEHOLDER_FILL: egui::Color32 = egui::Color32::BLACK;
/// Row-thumbnail aspect ratio — always the emulator's own aspect-corrected
/// display shape ([`crate::TARGET_ASPECT`]), never the framebuffer's raw
/// pixel aspect: buffer pixels aren't square (the CoCo 3 canonical raster is
/// 640×240 — 1:2 pixels), so drawing at the texture's own aspect would
/// stretch the picture (`draw_row_thumbnail`).
const THUMBNAIL_ASPECT: f32 = crate::TARGET_ASPECT;
/// Inner padding of one list row's frame.
const ROW_MARGIN: f32 = 8.0;
/// Corner rounding of a list row's selection/hover frame.
const ROW_CORNER_RADIUS: f32 = 4.0;

/// List-row / detail-pane status labels for the three machine states
/// (Powered Off / Running / Suspended). Not stored in the definition file
/// ("Decisions" — "Runtime status … is never persisted") — a function of
/// [`MachineEntry::vm`] and
/// [`MachineEntry::suspended`] at draw time (Suspended's persistence is the
/// [`SUSPEND_STATE_FILE`] itself), see [`vm_status_label`].
const STATUS_RUNNING: &str = "Running";
const STATUS_SUSPENDED: &str = "Suspended";
const STATUS_POWERED_OFF: &str = "Powered Off";

/// Vertical gap between sections of the detail pane.
const DETAIL_SECTION_GAP: f32 = 12.0;

/// Inner margin of the detail/edit pane, in egui logical points.
const DETAIL_PANE_MARGIN: i8 = 10;

/// Error text for Create/Save when [`ManagerApp::machines_dir`] is `None`
/// (no home directory — `paths::config_dir` docs).
pub(crate) const NO_CONFIG_DIR: &str = "no config directory available";

/// [`NO_CONFIG_DIR`]'s sibling for [`ManagerApp::artifacts_root`] — a
/// different directory (`machine_def::artifacts_root` under
/// `paths::data_dir`), reported by Suspend/Resume, which cannot work
/// without somewhere to keep the frozen state.
const NO_DATA_DIR: &str = "no data directory available";

/// One machine-list entry: a slug (file stem, also the identity used for
/// save/rename bookkeeping — `machine_def.rs` "Identity = slug") plus its
/// parsed definition. Always valid: a definition that fails to load or
/// validate is fatal at startup (`machine_def::load_all`), so no entry
/// carries an error.
pub struct MachineEntry {
    pub slug: String,
    pub def: machine_def::MachineDef,
    /// The running VM, once [`ManagerApp::start_vm`] has launched it —
    /// `None` means Powered Off (or Suspended with its window closed, see
    /// [`Self::suspended`]). Boxed: `CocoApp` is a large struct (the whole
    /// machine plus every UI dialog's state), and every `MachineEntry` pays
    /// its size even when stopped.
    pub vm: Option<Box<CocoApp>>,
    /// Whether this machine is Suspended — frozen to its artifact dir's
    /// [`SUSPEND_STATE_FILE`]. The file is the persistent truth
    /// ([`ManagerApp::new`] seeds this flag from its existence); the flag is
    /// the per-frame mirror so drawing never stats the filesystem.
    /// While suspended the VM object may still be alive (paused, window
    /// open) or already dropped (window closed) — both draw as Suspended.
    pub(crate) suspended: bool,
    /// The transport row's error channel: the message from the last failed
    /// Start, Suspend, Resume, or "Show config in …", shown in the detail
    /// pane until the next attempt or a fresh selection — the lifecycle
    /// analog of [`ManagerApp::save_error`].
    pub launch_error: Option<String>,
    /// Saved-preview texture for a *suspended* machine whose VM window is
    /// closed (its artifact dir's [`THUMBNAIL_FILE`], written at suspend
    /// time), loaded lazily on first row draw. Pure cache — never required
    /// state; a missing/undecodable file leaves the placeholder.
    /// `pub(crate)` for `ui_tests.rs` assertions.
    pub(crate) thumbnail: Option<egui::TextureHandle>,
    /// Whether a [`Self::thumbnail`] load was already attempted, so a
    /// machine with no thumbnail file doesn't retry the filesystem every
    /// frame. Cleared (with `thumbnail`) whenever a fresh PNG is written.
    thumbnail_load_attempted: bool,
    /// Whether an evicted preview can reload. False after a missing or bad file.
    thumbnail_known_available: bool,
    /// Manager-local use stamp for deterministic saved-preview LRU eviction.
    thumbnail_last_used: u64,
    /// Runtime-only identity for egui's immediate viewport. It stays stable
    /// when the persisted machine slug changes.
    window_session: u64,
}

const FIRST_WINDOW_SESSION: u64 = 1;
static NEXT_WINDOW_SESSION: AtomicU64 = AtomicU64::new(FIRST_WINDOW_SESSION);

impl MachineEntry {
    /// `slug` + a freshly loaded/created `def`, with no VM running and no
    /// stale launch error — the state every entry starts in.
    pub(crate) fn new(slug: String, def: machine_def::MachineDef) -> Self {
        Self {
            slug,
            def,
            vm: None,
            suspended: false,
            launch_error: None,
            thumbnail: None,
            thumbnail_load_attempted: false,
            thumbnail_known_available: false,
            thumbnail_last_used: 0,
            window_session: NEXT_WINDOW_SESSION.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Whether this machine is currently Running — a live VM, not paused
    /// for Suspend.
    pub(crate) fn is_running(&self) -> bool {
        self.vm.is_some() && !self.suspended
    }

    /// Whether this machine is anything other than Powered Off — Running or
    /// Suspended.
    pub(crate) fn is_alive(&self) -> bool {
        self.vm.is_some() || self.suspended
    }

    /// Whether Play has something to do: the machine isn't already Running.
    pub(crate) fn is_startable(&self) -> bool {
        !self.is_running()
    }
}

/// The three-state status `entry` implies right now. A debugger-paused VM
/// (no suspended flag) still reads Running — pause isn't a lifecycle state.
fn vm_status_label(entry: &MachineEntry) -> &'static str {
    if entry.suspended {
        STATUS_SUSPENDED
    } else if entry.vm.is_some() {
        STATUS_RUNNING
    } else {
        STATUS_POWERED_OFF
    }
}

/// File name of a suspended machine's saved screen preview, inside its
/// artifact directory (`machine_def::artifacts_root()/<slug>`) — written at
/// suspend time so the row keeps showing the frozen frame after the VM
/// window closes (and across manager restarts).
const THUMBNAIL_FILE: &str = "thumbnail.png";

/// File name of a suspended machine's frozen state (the save-states
/// `.ccstate` format — `coco_core::snapshot` through `CocoApp::save_state_to`),
/// inside its artifact directory. Its existence IS the persistent Suspended
/// state: written by Suspend, consumed by Resume (deleted strictly before
/// the machine is declared Running — resuming discards the frozen copy,
/// VirtualBox-style) and by Stop (powering off a suspended machine discards
/// it too). Neither lets the in-memory state contradict a file it failed to
/// delete: a resume that cannot consume the file fails outright, and a stop
/// that cannot discard it leaves the entry Suspended
/// (`manager::lifecycle`) — otherwise [`ManagerApp::new`]'s
/// existence check would resurrect a checkpoint the running session had
/// already moved past.
const SUSPEND_STATE_FILE: &str = "suspended.ccstate";

/// `<artifacts_root>/<slug>/suspended.ccstate`.
fn suspend_state_path(artifacts_root: &Path, slug: &str) -> PathBuf {
    artifacts_root.join(slug).join(SUSPEND_STATE_FILE)
}

/// Write `rgba` (`w`×`h`) as `dir/thumbnail.png`, tmp-then-rename so a crash
/// mid-write can't leave a torn file. A uniformly black frame is skipped
/// when a previous thumbnail exists, so a blanked display doesn't clobber it.
fn write_thumbnail_png(dir: &Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let final_path = dir.join(THUMBNAIL_FILE);
    let all_black = rgba
        .as_chunks::<{ coco_core::video::BYTES_PER_PIXEL }>()
        .0
        .iter()
        .all(|px| px[0] == 0 && px[1] == 0 && px[2] == 0);
    if all_black && final_path.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let image = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or_else(|| {
        format!(
            "framebuffer geometry mismatch: {w}x{h} vs {} bytes",
            rgba.len()
        )
    })?;
    let tmp_path = dir.join(format!("{THUMBNAIL_FILE}.tmp"));
    image
        .save_with_format(&tmp_path, image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))
}

pub struct ManagerApp {
    #[cfg(feature = "perf")]
    perf_scenario: Option<perf_scenarios::ScenarioRun>,
    /// Single host gamepad backend retained across every VM lifecycle.
    gamepad: crate::joy::SharedGamepad,
    /// The right pane's photo while nothing is selected, its timer, and
    /// their settings (`manager/welcome_image.rs`).
    pub(crate) welcome_image: welcome_image::WelcomeImage,
    /// Directory new/edited definitions are saved to. `None` when no home
    /// directory exists (`paths::config_dir` docs) — Save/Create then report
    /// the problem in place rather than silently doing nothing.
    machines_dir: Option<PathBuf>,
    /// Installed ROM directory (`paths::roms_dir()`) the detail pane's
    /// ROMs group resolves stock images under; `None` when no home
    /// directory exists, and in tests, which must never read the real one.
    pub(crate) roms_dir: Option<PathBuf>,
    /// Installed cartridge directory (`paths::cartridges_dir()`) the About
    /// window counts images in; `None` like [`Self::roms_dir`], and in tests.
    pub(crate) cartridges_dir: Option<PathBuf>,
    /// Root of the per-machine artifact directories
    /// (`machine_def::artifacts_root()`), where each entry's
    /// [`THUMBNAIL_FILE`] and [`SUSPEND_STATE_FILE`] live under
    /// `<root>/<slug>`. Injected like `machines_dir` so tests use a temp
    /// dir, never the real data dir; `None` disables thumbnail persistence
    /// and Suspend/Resume entirely (they error with [`NO_DATA_DIR`]).
    artifacts_root: Option<PathBuf>,
    /// `pub(crate)`: `ui_tests.rs` asserts on the list contents directly —
    /// `ManagerApp` lives in this module, so plain private fields (as
    /// `CocoApp` in the crate root uses) aren't visible from that sibling
    /// module.
    pub(crate) entries: Vec<MachineEntry>,
    /// Which rows are selected — zero, one, or many (`manager/selection.rs`).
    /// `pub(crate)`: `ui_tests` asserts on it directly, same reason as
    /// `entries`.
    pub(crate) selection: Selection,
    /// The right pane's edit state for [`Selection::single`]. `None` while
    /// nothing, or more than one row, is selected — edit state only makes
    /// sense for one machine at a time, so every selection change that
    /// leaves the count at anything but 1 clears this
    /// (`manager/list.rs`'s click handler).
    edit: Option<EditState>,
    /// Focus the detail pane's Name field on its next draw — set by "New…"
    /// so the natural next gesture after creating is typing the real name.
    focus_name: bool,
    /// Message from the last failed Save, shown under the Save/Revert row
    /// until the next attempt or a fresh selection.
    save_error: Option<String>,
    /// Slugs of the entries a "Delete…" (single- or multi-row) was clicked
    /// for — a confirmation modal ([`Self::draw_delete_confirmation`]) shows
    /// while this is non-empty. Slugs, not indices: rows can shift under a
    /// pending confirmation (another instance's file picked up on a future
    /// reload, a sort change, or a Create landing elsewhere), and deleting the
    /// wrong row is the one mistake this dialog exists to prevent.
    pending_delete: Vec<String>,
    /// Message from the last failed delete, shown inside the confirmation
    /// modal (which stays open for another try or a Cancel).
    delete_error: Option<String>,
    /// Per-machine choices and retryable failures while confirming app exit.
    exit: exit::ExitState,
    /// The app's built-in MCP server (`crate::control`), servicing an AI
    /// driving a VM over HTTP. `None` when disabled (`--control-port 0`) or
    /// its bind failed at startup.
    control: Option<crate::control::ControlServer>,
    /// Control requests deferred until the VM they target finishes some work
    /// (`manager::control`), resolved once per frame after VMs have stepped.
    pending: Vec<control::PendingControl>,
    /// A committed Name-field edit waiting for the next frame's pre-draw
    /// rename transaction.
    pending_rename: Option<rename::PendingRename>,
    /// The first-run asset download dialog (`manager/assets.rs`), open while
    /// `Some`. `pub(crate)`: `ui_tests.rs` seeds and asserts on it, like `entries`.
    pub(crate) asset_dialog: Option<assets::AssetDialog>,
    /// Global toolbar caption toggle, pushed to every open VM window each
    /// frame (`manager/vm_windows.rs`) so Settings changes apply everywhere.
    pub(crate) toolbar_icons_only: bool,
    /// True when a CLI flag or env var supplied `toolbar_icons_only`
    /// (`Config::toolbar_icons_only_overridden`); Settings then skips the
    /// live apply on save so the override keeps winning until restart.
    pub(crate) toolbar_icons_only_overridden: bool,
    /// True when a CLI flag or env var supplied `control_port`
    /// (`Config::control_port_overridden`); Settings then leaves the live
    /// listener alone on save so the override keeps winning until restart.
    pub(crate) control_port_overridden: bool,
    /// Re-levels the global log subscriber on a Settings save
    /// (`startup::setup_logging`); `None` outside the app unless a test seeds one.
    pub(crate) log_reload: Option<crate::startup::LogReload>,
    /// `control_port_overridden`'s counterpart for `log_level`.
    pub(crate) log_level_overridden: bool,
    /// Global status-bar readout toggle, pushed to every open VM window
    /// each frame like `toolbar_icons_only`. The manager has no status bar
    /// of its own, so nothing here reads it.
    pub(crate) status_bar_icons_only: bool,
    /// `toolbar_icons_only_overridden`'s counterpart for `status_bar_icons_only`.
    pub(crate) status_bar_icons_only_overridden: bool,
    /// Rebindable hotkeys (`hotkeys.rs`): New machine fires here, the rest
    /// are pushed to every open VM window each frame like `toolbar_icons_only`.
    pub(crate) hotkeys: crate::hotkeys::Hotkeys,
    /// Where `config.toml` lives (`run::run`'s own `config_path`), for the
    /// Settings dialog to load and save. `None` when no home directory
    /// exists (`paths::config_dir` docs) — Settings then opens with the
    /// built-in defaults and reports the problem on Save instead.
    pub(crate) config_path: Option<PathBuf>,
    /// Current machine-list ordering, loaded from and written to
    /// `config.toml` through the list's sort controls.
    pub(crate) manager_sort: ManagerSort,
    /// The last failure to persist [`Self::manager_sort`], shown beside the
    /// controls while the live order remains in effect.
    sort_error: Option<String>,
    /// The Settings dialog (`manager/settings.rs`), open while `Some`.
    pub(crate) settings: Option<settings::SettingsDialog>,
    /// The About window ([`crate::about::window`]), opened from the toolbar's
    /// Help menu or [`Self::about_request`]. `pub(crate)` for `ui_tests`.
    pub(crate) show_about: bool,
    /// The About window's inventory line ([`crate::startup::inventory`]),
    /// counted when the window opens rather than every frame.
    about_inventory: String,
    /// Opens the About window from the macOS application menu.
    pub(crate) about_request: crate::about::AboutRequest,
    /// Monotonic clock for saved-preview LRU stamps.
    thumbnail_use_clock: u64,
    /// Synchronous preview decodes still available in this manager update.
    thumbnail_loads_remaining: usize,
    /// Row the machine list scrolls into view on its next draw (arrow keys).
    scroll_to_row: Option<usize>,
}

impl ManagerApp {
    /// All state is injected rather than loaded here, so tests can construct
    /// the manager without touching the user's real config/data directories.
    /// Each entry's Suspended flag is seeded from its
    /// [`SUSPEND_STATE_FILE`]'s existence. `control` is already bound (or
    /// `None`) — binding needs a `CreationContext`'s `egui::Context` for its
    /// wake closure, which only [`run`] has, so it happens there.
    #[cfg(test)]
    pub fn new(
        photo: Option<Photo>,
        machines_dir: Option<PathBuf>,
        artifacts_root: Option<PathBuf>,
        entries: Vec<MachineEntry>,
        control: Option<crate::control::ControlServer>,
    ) -> Self {
        Self::new_with_sort(
            photo,
            machines_dir,
            artifacts_root,
            entries,
            control,
            crate::config::DEFAULT_MANAGER_SORT,
        )
    }

    fn new_with_sort(
        photo: Option<Photo>,
        machines_dir: Option<PathBuf>,
        artifacts_root: Option<PathBuf>,
        mut entries: Vec<MachineEntry>,
        control: Option<crate::control::ControlServer>,
        manager_sort: ManagerSort,
    ) -> Self {
        if let Some(root) = &artifacts_root {
            for entry in &mut entries {
                entry.suspended = suspend_state_path(root, &entry.slug).is_file();
            }
        }
        sort::sort_entries(&mut entries, manager_sort);
        Self {
            #[cfg(feature = "perf")]
            perf_scenario: None,
            gamepad: crate::joy::SharedGamepad::new(),
            welcome_image: welcome_image::WelcomeImage::new(photo),
            machines_dir,
            roms_dir: None,
            cartridges_dir: None,
            artifacts_root,
            entries,
            selection: Selection::default(),
            edit: None,
            focus_name: false,
            save_error: None,
            pending_delete: Vec::new(),
            delete_error: None,
            exit: exit::ExitState::default(),
            control,
            pending: Vec::new(),
            pending_rename: None,
            asset_dialog: None,
            toolbar_icons_only: false,
            toolbar_icons_only_overridden: false,
            control_port_overridden: false,
            log_reload: None,
            log_level_overridden: false,
            status_bar_icons_only: false,
            status_bar_icons_only_overridden: false,
            hotkeys: crate::hotkeys::Hotkeys::default(),
            config_path: None,
            manager_sort,
            sort_error: None,
            settings: None,
            show_about: false,
            about_inventory: String::new(),
            about_request: crate::about::AboutRequest::default(),
            thumbnail_use_clock: 0,
            thumbnail_loads_remaining: thumbnails::THUMBNAIL_LOADS_PER_UPDATE,
            scroll_to_row: None,
        }
    }
}

impl eframe::App for ManagerApp {
    /// Flushes every running VM's dirty disks/tape on quit and folds each
    /// live VM's runtime into its persisted total, same as Stop. A flush
    /// failure is only logged — there's no dialog left to show it in.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.apply_pending_rename();
        for index in 0..self.entries.len() {
            self.fold_runtime_into_def(index);
            if let Some(mut vm) = self.entries[index].vm.take() {
                vm.stop_drivewire_host();
                if let Err(e) = vm.flush_media() {
                    tracing::warn!(
                        "could not flush media for '{}' on exit: {e}",
                        self.entries[index].slug
                    );
                }
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.handle_exit_request(ctx) {
            return;
        }
        #[cfg(feature = "perf")]
        self.drive_perf_scenario(ctx);
        crate::perf::initialize();
        let _perf = crate::perf::span(crate::perf::Stage::ManagerUpdate);
        self.service_gamepad(ctx);
        // Dialog-first phase: while the asset dialog is up, it is the
        // window's only content — the manager UI appears after a successful
        // download (Cancel quits the app, `manager/assets.rs`).
        if self.asset_dialog.is_some() {
            // The dialog-sized window has no room for About; drop the request.
            self.about_request.take();
            self.draw_asset_dialog(ctx);
            self.draw_exit_confirmation(ctx);
            return;
        }

        self.poll_about_request(ctx);
        self.welcome_image.service(ctx, self.selection.is_empty());
        self.thumbnail_loads_remaining = thumbnails::THUMBNAIL_LOADS_PER_UPDATE;

        // Apply a committed rename before any panel draws — row indices must
        // stay stable for the frame.
        self.apply_pending_rename();

        // The New machine hotkey triggers New…; only fires with the manager window focused.
        // Settings' hotkey capture needs the raw press, so nothing here takes keys meanwhile.
        if self.settings.is_none() && !self.exit.is_pending() {
            let new_machine = self.hotkeys.new_machine;
            if ctx.input_mut(|i| new_machine.consume(i)) {
                self.create_machine_now();
            }
            self.handle_list_shortcuts(ctx);
        }

        let toolbar_frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(crate::TOOLBAR_PANEL_MARGIN_X, crate::TOOLBAR_PANEL_MARGIN_Y),
        );
        egui::TopBottomPanel::top("manager_toolbar")
            .frame(toolbar_frame)
            .exact_height(crate::toolbar_height(self.toolbar_icons_only))
            .show(ctx, |ui| {
                self.draw_toolbar(ui);
            });

        egui::SidePanel::left("manager_machine_list")
            .resizable(true)
            .default_width(LIST_DEFAULT_WIDTH)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show(ctx, |ui| {
                self.draw_machine_list(ui);
            });

        // Detail form for a single selection, bulk pane for many, or the
        // welcome instructions and image when nothing's selected.
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.selection.is_empty() {
                welcome::draw(ui, &mut self.welcome_image, !self.entries.is_empty());
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::same(DETAIL_PANE_MARGIN))
                        .show(ui, |ui| match self.selection.single() {
                            Some(index) => self.draw_detail(ui, index),
                            None => self.draw_bulk_detail(ui),
                        });
                });
            }
        });

        if !self.exit.is_pending() {
            self.draw_delete_confirmation(ctx);
            self.draw_settings_dialog(ctx);
            if self.show_about {
                crate::about::window(ctx, &mut self.show_about, &self.about_inventory);
            }
        }
        self.drain_control();
        self.draw_running_vms(ctx);
        self.resolve_control_pending(ctx);
        self.draw_exit_confirmation(ctx);
    }
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod tests;
