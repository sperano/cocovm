//! The CocoVM manager window: the VirtualBox/Parallels-style main window
//! `coco` always opens. Toolbar across the top, machine list down the left
//! (one row per `config_dir()/machines/<slug>.toml`, `machine_def.rs`), and
//! a detail/edit pane on the right for the selected machine — or, with no machine
//! selected, a random photo asset filling the pane.
//!
//! The detail pane's Start button calls `crate::launch_machine`. Once a
//! `MachineEntry` holds a running `CocoApp`, `ManagerApp::update` opens it in
//! its own native OS window every frame—an *immediate viewport*, like the
//! printer-paper window in `paper_view::PaperWindow`. All VM state stays on
//! the main thread. Each viewport's child `egui::Context` delivers keyboard
//! and mouse input for that window, so egui handles focus routing
//! ("DECIDED: in-process, one native window per running VM"). The app always
//! opens this manager window. A future CLI will build on the manager's own
//! machine definitions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;

use crate::photo_view::Photo;
use crate::{CocoApp, machine_def, new_vm};

use selection::Selection;

pub(crate) mod assets;
mod bulk;
mod control;
mod delete;
mod detail;
mod detail_map;
mod lifecycle;
mod list;
#[cfg(feature = "perf")]
mod perf_scenarios;
mod rename;
mod run;
mod selection;
mod settings;
mod thumbnails;
mod toolbar;
mod vm_windows;

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

/// The "select all rows" shortcut (⌘A/Ctrl+A) for the machine list —
/// consumed only when no widget owns the keyboard
/// ([`eframe::App::update`]'s `ctx.wants_keyboard_input()` guard), so the
/// detail pane's own text fields (the Name field, a future search box)
/// keep their native select-all instead of it being hijacked into a
/// row-selection command.
const SELECT_ALL_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A);

/// Error text for Create/Save when [`ManagerApp::machines_dir`] is `None`
/// (no home directory — `paths::config_dir` docs).
const NO_CONFIG_DIR: &str = "no config directory available";

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
    /// Start, Suspend, or Resume, shown in the detail pane until the next
    /// attempt or a fresh selection — the lifecycle analog of
    /// [`ManagerApp::save_error`].
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
        .chunks_exact(4)
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

/// The detail pane's working state for the selected entry: the shared
/// [`new_vm::MachineForm`] over its definition, auto-saved on every change
/// (macOS System Settings style — no Save/Revert).
struct EditState {
    /// Which entry this state belongs to — a mismatch (a different row was
    /// clicked) means it must be reseeded before it's shown again.
    slug: String,
    /// The Name field's draft. Unlike the form, it only commits (saves, and
    /// migrates the slug — [`ManagerApp::commit_name`]) on focus loss/Enter,
    /// so half-typed names aren't saved keystroke by keystroke.
    name: String,
    form: new_vm::MachineForm,
    /// The definition the form's picks last packed into ([`ManagerApp::pack_def`]) —
    /// the auto-save baseline. Seeded from the freshly seeded form (NOT
    /// from the entry's definition): packing normalizes (explicit `vdg`,
    /// re-seated MPI slots, dropped conflicting flags), and merely
    /// selecting a row must never rewrite a hand-edited file. Only a real
    /// user change makes the repack differ from this and triggers a save.
    packed: machine_def::MachineDef,
}

pub struct ManagerApp {
    #[cfg(feature = "perf")]
    perf_scenario: Option<perf_scenarios::ScenarioRun>,
    /// Decoded photo pending its first-frame texture upload.
    photo: Option<Photo>,
    /// The uploaded photo texture, once a frame has run.
    photo_texture: Option<egui::TextureHandle>,
    /// Directory new/edited definitions are saved to. `None` when no home
    /// directory exists (`paths::config_dir` docs) — Save/Create then report
    /// the problem in place rather than silently doing nothing.
    machines_dir: Option<PathBuf>,
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
    /// reload, a Create landing before it alphabetically), and deleting the
    /// wrong row is the one mistake this dialog exists to prevent.
    pending_delete: Vec<String>,
    /// Message from the last failed delete, shown inside the confirmation
    /// modal (which stays open for another try or a Cancel).
    delete_error: Option<String>,
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
    /// Global toolbar caption toggle, pushed to every running VM
    /// (`lifecycle::apply_icons_only_to_running_vms`).
    pub(crate) toolbar_icons_only: bool,
    /// True when a CLI flag or env var supplied `toolbar_icons_only`
    /// (`Config::toolbar_icons_only_overridden`); Settings then skips the
    /// live apply on save so the override keeps winning until restart.
    pub(crate) toolbar_icons_only_overridden: bool,
    /// Global status-bar readout toggle, pushed to every running VM like
    /// `toolbar_icons_only`. The manager has no status bar of its own, so
    /// nothing here reads it.
    pub(crate) status_bar_icons_only: bool,
    /// `toolbar_icons_only_overridden`'s counterpart for `status_bar_icons_only`.
    pub(crate) status_bar_icons_only_overridden: bool,
    /// Where `config.toml` lives (`run::run`'s own `config_path`), for the
    /// Settings dialog to load and save. `None` when no home directory
    /// exists (`paths::config_dir` docs) — Settings then opens with the
    /// built-in defaults and reports the problem on Save instead.
    pub(crate) config_path: Option<PathBuf>,
    /// The Settings dialog (`manager/settings.rs`), open while `Some`.
    pub(crate) settings: Option<settings::SettingsDialog>,
}

impl ManagerApp {
    /// All state is injected rather than loaded here, so tests can construct
    /// the manager without touching the user's real config/data directories.
    /// Each entry's Suspended flag is seeded from its
    /// [`SUSPEND_STATE_FILE`]'s existence. `control` is already bound (or
    /// `None`) — binding needs a `CreationContext`'s `egui::Context` for its
    /// wake closure, which only [`run`] has, so it happens there.
    pub fn new(
        photo: Option<Photo>,
        machines_dir: Option<PathBuf>,
        artifacts_root: Option<PathBuf>,
        mut entries: Vec<MachineEntry>,
        control: Option<crate::control::ControlServer>,
    ) -> Self {
        if let Some(root) = &artifacts_root {
            for entry in &mut entries {
                entry.suspended = suspend_state_path(root, &entry.slug).is_file();
            }
        }
        Self {
            #[cfg(feature = "perf")]
            perf_scenario: None,
            photo,
            photo_texture: None,
            machines_dir,
            artifacts_root,
            entries,
            selection: Selection::default(),
            edit: None,
            focus_name: false,
            save_error: None,
            pending_delete: Vec::new(),
            delete_error: None,
            control,
            pending: Vec::new(),
            pending_rename: None,
            asset_dialog: None,
            toolbar_icons_only: false,
            toolbar_icons_only_overridden: false,
            status_bar_icons_only: false,
            status_bar_icons_only_overridden: false,
            config_path: None,
            settings: None,
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
            if let Some(mut vm) = self.entries[index].vm.take()
                && let Err(e) = vm.flush_media()
            {
                tracing::warn!(
                    "could not flush media for '{}' on exit: {e}",
                    self.entries[index].slug
                );
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(feature = "perf")]
        self.drive_perf_scenario(ctx);
        crate::perf::initialize();
        let _perf = crate::perf::span(crate::perf::Stage::ManagerUpdate);
        // Dialog-first phase: while the asset dialog is up, it is the
        // window's only content — the manager UI appears after a successful
        // download (Cancel quits the app, `manager/assets.rs`).
        if self.asset_dialog.is_some() {
            self.draw_asset_dialog(ctx);
            return;
        }

        if let Some(photo) = self.photo.take() {
            self.photo_texture =
                Some(ctx.load_texture(&photo.title, photo.pixels, egui::TextureOptions::LINEAR));
        }

        // Apply a committed rename before any panel draws — row indices must
        // stay stable for the frame.
        self.apply_pending_rename();

        // ⌘N/Ctrl+N triggers New…; only fires with the manager window focused.
        if ctx.input_mut(|i| i.consume_shortcut(&new_vm::NEW_MACHINE_SHORTCUT)) {
            self.create_machine_now();
        }

        // ⌘A/Ctrl+A selects every row, unless a widget already owns the keyboard.
        if !ctx.wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_shortcut(&SELECT_ALL_SHORTCUT))
        {
            self.select_all_rows();
        }

        egui::TopBottomPanel::top("manager_toolbar").show(ctx, |ui| {
            self.draw_toolbar(ui);
        });

        egui::SidePanel::left("manager_machine_list")
            .resizable(true)
            .default_width(LIST_DEFAULT_WIDTH)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show(ctx, |ui| {
                self.draw_machine_list(ui);
            });

        // Detail form for a single selection, bulk pane for many, or a
        // random photo when nothing's selected.
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.selection.is_empty() {
                if let Some(texture) = &self.photo_texture {
                    ui.centered_and_justified(|ui| {
                        ui.add(
                            egui::Image::new(texture)
                                .max_size(ui.available_size())
                                .maintain_aspect_ratio(true),
                        );
                    });
                }
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

        self.draw_delete_confirmation(ctx);
        self.draw_settings_dialog(ctx);
        self.drain_control();
        self.draw_running_vms(ctx);
        self.resolve_control_pending(ctx);
    }
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod tests;
