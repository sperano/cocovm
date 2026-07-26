//! The CocoVM manager window: the VirtualBox/Parallels-style main window a
//! bare `coco` (no CLI arguments) opens instead of booting a machine
//! directly. Toolbar across the top, machine list down the left (one row per
//! `config_dir()/machines/<slug>.toml`, `machine_def.rs`), and a detail/edit
//! pane on the right for the selected machine — or, with no machine
//! selected, a random photo asset filling the pane.
//!
//! Launching a machine (`plan-machine-persistence.md` step 5) is wired up:
//! the detail pane's Start button calls `crate::launch_machine` and, once a
//! `MachineEntry` holds a running `CocoApp`, `ManagerApp::update` opens it in
//! its own native OS window every frame — an *immediate viewport*, the same
//! pattern `paper_view::PaperWindow` uses for the printer-paper window (see
//! that module's doc comment). All VM state stays on the main thread; each
//! viewport's own child `egui::Context` delivers that window's keyboard/
//! mouse input, so focus routing comes for free from egui
//! (`docs/plan-machine-persistence.md` "DECIDED: in-process, one native
//! window per running VM"). The direct-boot emulator (`CocoApp`) is
//! otherwise untouched and still serves every CLI invocation with arguments.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui;

use crate::photo_view::{self, Photo};
use crate::{machine_def, new_vm, CocoApp};

mod detail;
mod lifecycle;
mod list;
mod thumbnails;
mod vm_windows;

/// Manager window size at first open.
const WINDOW_SIZE: [f32; 2] = [1080.0, 720.0];

/// Machine-list panel: width at first open and the draggable divider's range.
const LIST_DEFAULT_WIDTH: f32 = 260.0;
const LIST_MIN_WIDTH: f32 = 160.0;
const LIST_MAX_WIDTH: f32 = 520.0;

/// Corner rounding of the row thumbnail placeholder itself — distinct from
/// [`ROW_CORNER_RADIUS`], the row's own selection/hover frame.
const THUMBNAIL_CORNER_RADIUS: f32 = 2.0;
/// Fill of the row thumbnail placeholder — shown as a stopped machine's
/// whole thumbnail (until its saved [`THUMBNAIL_FILE`] loads, if one
/// exists), and as a running one's letterbox background before its texture
/// is uploaded/when the texture's aspect doesn't exactly fill the allocated
/// rect.
const THUMBNAIL_PLACEHOLDER_FILL: egui::Color32 = egui::Color32::from_gray(30);
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

/// Hover text of the always-disabled Suspend action (detail-pane button and
/// row context-menu item alike) — the *heavy* freeze that ships with the
/// save-states milestone (`docs/plan-save-states.md`); the disabled control
/// teaches the model before the feature exists.
const SUSPEND_DISABLED_HOVER: &str =
    "Freeze this machine to disk and free it — resume later, even after \
     quitting or on another computer. Coming with save-states.";

/// List-row / detail-pane status labels. Never persisted
/// (`plan-machine-persistence.md` "Decisions" — "Runtime status … is never
/// persisted"): purely a function of [`MachineEntry::vm`] at draw time, see
/// [`vm_status_label`].
const STATUS_RUNNING: &str = "Running";
const STATUS_PAUSED: &str = "Paused";
const STATUS_STOPPED: &str = "Stopped";

/// Vertical gap between sections of the detail pane.
const DETAIL_SECTION_GAP: f32 = 12.0;

/// Cassette-deck transport glyphs (all in egui's documented built-in emoji
/// set, `egui/src/lib.rs` "special emojis"). The deck metaphor is applied
/// only to the machine's *execution* — run, freeze, power off — where it's
/// honest; disk-level Suspend and the console Reset button deliberately stay
/// ordinary labeled buttons outside the transport row (user decision
/// 2026-07-16, after the "light pause vs dump-to-disk suspend" discussion —
/// state model in `docs/plan-machine-persistence.md`).
pub(crate) const PLAY_GLYPH: &str = "▶";
pub(crate) const PAUSE_GLYPH: &str = "⏸";
pub(crate) const STOP_GLYPH: &str = "⏹";

/// Error text for Create/Save when [`ManagerApp::machines_dir`] is `None`
/// (no home directory — `paths::config_dir` docs).
const NO_CONFIG_DIR: &str = "no config directory available";

/// One machine-list entry: a slug (file stem, also the identity used for
/// save/rename bookkeeping — `machine_def.rs` "Identity = slug") plus its
/// parsed definition. Always valid: a definition that fails to load or
/// validate is fatal at startup (`machine_def::load_all`), so no entry
/// carries an error.
pub struct MachineEntry {
    pub slug: String,
    pub def: machine_def::MachineDef,
    /// The running VM, once [`ManagerApp::start_vm`] has launched it —
    /// `None` means Stopped. Boxed: `CocoApp` is a large struct (the whole
    /// machine plus every UI dialog's state), and every `MachineEntry` pays
    /// its size even when stopped.
    pub vm: Option<Box<CocoApp>>,
    /// Message from the last failed Start, shown in the detail pane until
    /// the next Start attempt or a fresh selection — the launch-time analog
    /// of [`ManagerApp::save_error`].
    pub launch_error: Option<String>,
    /// Saved-preview texture for a *stopped* machine (its artifact dir's
    /// [`THUMBNAIL_FILE`]), loaded lazily on first row draw. Pure cache —
    /// never required state; a missing/undecodable file just leaves the
    /// placeholder. `pub(crate)` for `ui_tests.rs` assertions.
    pub(crate) thumbnail: Option<egui::TextureHandle>,
    /// Whether a [`Self::thumbnail`] load was already attempted, so a
    /// machine with no thumbnail file doesn't retry the filesystem every
    /// frame. Cleared (with `thumbnail`) whenever a fresh PNG is written.
    thumbnail_load_attempted: bool,
    /// When this entry's *running* VM last had its `thumbnail.png`
    /// refreshed — drives the [`THUMBNAIL_REFRESH`] crash-insurance cadence.
    last_thumbnail_write: Option<Instant>,
    /// The machine was renamed while running, so its `<slug>.toml`/artifact
    /// dir couldn't follow the new name yet (the running VM writes
    /// `thumbnail.png` into the artifact dir by path — renaming under it
    /// races). [`ManagerApp::apply_pending_renames`] migrates once the VM is
    /// gone. Not persisted: quitting with this set leaves the stale slug
    /// until the next rename commit.
    rename_pending: bool,
}

impl MachineEntry {
    /// `slug` + a freshly loaded/created `def`, with no VM running yet and
    /// no stale launch error — the state every entry starts in, whether
    /// loaded from disk ([`ManagerApp`]'s `run`) or just created (`Self`'s
    /// callers previously wrote out the `vm`/`launch_error` fields by hand;
    /// this constructor is what keeps that from drifting as more per-entry
    /// runtime state gets added later).
    pub(crate) fn new(slug: String, def: machine_def::MachineDef) -> Self {
        Self {
            slug,
            def,
            vm: None,
            launch_error: None,
            thumbnail: None,
            thumbnail_load_attempted: false,
            last_thumbnail_write: None,
            rename_pending: false,
        }
    }
}

/// The status [`MachineEntry::vm`] implies right now — never persisted, see
/// [`STATUS_RUNNING`]'s doc.
fn vm_status_label(entry: &MachineEntry) -> &'static str {
    match &entry.vm {
        Some(vm) if vm.is_running() => STATUS_RUNNING,
        Some(_) => STATUS_PAUSED,
        None => STATUS_STOPPED,
    }
}

/// File name of a stopped machine's saved screen preview, inside its
/// artifact directory (`machine_def::artifacts_root()/<slug>`).
const THUMBNAIL_FILE: &str = "thumbnail.png";

/// How often a running VM's `thumbnail.png` is refreshed on disk. Stop and
/// manager-exit both do a final write regardless — this periodic one is
/// crash insurance, so a force-killed process still shows a recent preview
/// on the next launch instead of nothing.
const THUMBNAIL_REFRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// Write `rgba` (`w`×`h`) as `dir/thumbnail.png`. Same tmp-then-rename
/// pattern as `machine_def::save`, so a crash mid-write can never leave a
/// torn PNG behind. A uniformly *black* frame is skipped whenever a previous
/// thumbnail exists: stopping during a blanked display (mode switch, blank
/// screen) would otherwise replace a useful preview with a black rectangle.
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
        format!("framebuffer geometry mismatch: {w}x{h} vs {} bytes", rgba.len())
    })?;
    let tmp_path = dir.join(format!("{THUMBNAIL_FILE}.tmp"));
    image
        .save_with_format(&tmp_path, image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|e| format!("{}: {e}", final_path.display()))
}

/// The detail pane's working state for the selected entry: the shared
/// [`new_vm::MachineForm`] over its definition, auto-saved on every change
/// (macOS System Settings style — no Save/Revert, user decision
/// 2026-07-24).
struct EditState {
    /// Which entry this state belongs to — a mismatch (a different row was
    /// clicked) means it must be reseeded before it's shown again.
    slug: String,
    /// The Name field's draft. Unlike the form, it only commits (saves, and
    /// migrates the slug — [`ManagerApp::commit_name`]) on focus loss/Enter,
    /// so half-typed names aren't saved keystroke by keystroke.
    name: String,
    form: new_vm::MachineForm,
    /// The definition the form's picks last packed into ([`pack_def`]) —
    /// the auto-save baseline. Seeded from the freshly seeded form (NOT
    /// from the entry's definition): packing normalizes (explicit `vdg`,
    /// re-seated MPI slots, dropped conflicting flags), and merely
    /// selecting a row must never rewrite a hand-edited file. Only a real
    /// user change makes the repack differ from this and triggers a save.
    packed: machine_def::MachineDef,
}

pub struct ManagerApp {
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
    /// [`THUMBNAIL_FILE`] lives under `<root>/<slug>`. Injected like
    /// `machines_dir` so tests use a temp dir, never the real data dir;
    /// `None` disables thumbnail persistence entirely.
    artifacts_root: Option<PathBuf>,
    /// `pub(crate)`: `ui_tests.rs` asserts on the list contents directly —
    /// `ManagerApp` lives in this module, so plain private fields (as
    /// `CocoApp` in the crate root uses) aren't visible from that sibling
    /// module.
    pub(crate) entries: Vec<MachineEntry>,
    pub(crate) selected: Option<usize>,
    /// The right pane's edit state for `selected`. `None` while nothing is
    /// selected.
    edit: Option<EditState>,
    /// Focus the detail pane's Name field on its next draw — set by "New…"
    /// so the natural next gesture after creating is typing the real name.
    focus_name: bool,
    /// Message from the last failed Save, shown under the Save/Revert row
    /// until the next attempt or a fresh selection.
    save_error: Option<String>,
    /// Slug of the entry a context menu's "Delete…" was clicked for — a
    /// confirmation modal ([`Self::draw_delete_confirmation`]) shows while
    /// this is `Some`. Slug, not index: rows can shift under a pending
    /// confirmation (another instance's file picked up on a future reload,
    /// a Create landing before it alphabetically), and deleting the wrong
    /// row is the one mistake this dialog exists to prevent.
    pending_delete: Option<String>,
    /// Message from the last failed delete, shown inside the confirmation
    /// modal (which stays open for another try or a Cancel).
    delete_error: Option<String>,
}

impl ManagerApp {
    /// `photo`, `machines_dir`, `artifacts_root`, and `entries` are all
    /// injected (rather than loaded here) so tests can construct the manager
    /// without touching the user's real config/data directories.
    pub fn new(
        photo: Option<Photo>,
        machines_dir: Option<PathBuf>,
        artifacts_root: Option<PathBuf>,
        entries: Vec<MachineEntry>,
    ) -> Self {
        Self {
            photo,
            photo_texture: None,
            machines_dir,
            artifacts_root,
            entries,
            selected: None,
            edit: None,
            focus_name: false,
            save_error: None,
            pending_delete: None,
            delete_error: None,
        }
    }
}

impl eframe::App for ManagerApp {
    /// Flush every running VM's dirty disks/tape on quit — the manager
    /// window is the root viewport, so closing it closes every VM at once
    /// (`docs/plan-machine-persistence.md` "Lifetime rule"); this mirrors
    /// `CocoApp::on_exit`'s own contract for each of them.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for i in 0..self.entries.len() {
            // Final preview before the write-back: quitting with VMs still
            // running is the most common way a stopped row would otherwise
            // lose its saved thumbnail.
            self.write_entry_thumbnail(i);
            if let Some(vm) = self.entries[i].vm.as_mut() {
                vm.flush_media();
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(photo) = self.photo.take() {
            self.photo_texture =
                Some(ctx.load_texture(&photo.title, photo.pixels, egui::TextureOptions::LINEAR));
        }

        // Deferred slug migrations first, before any panel draws — row
        // indices must stay stable for the whole frame.
        self.apply_pending_renames();

        // ⌘N / Ctrl+N = the toolbar's "New…". Each running VM window is its
        // own viewport with its own input stream, so this only fires with
        // the manager window focused.
        if ctx.input_mut(|i| i.consume_shortcut(&new_vm::NEW_MACHINE_SHORTCUT)) {
            self.create_machine_now();
        }

        // Toolbar: the manager actions. "Settings"/"Help" are still inert
        // scaffolding.
        egui::TopBottomPanel::top("manager_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Toolbar buttons show the shortcut on hover (inline
                // shortcut text is a menu-row convention, not a toolbar one).
                if ui
                    .button("New…")
                    .on_hover_text(ctx.format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT))
                    .clicked()
                {
                    self.create_machine_now();
                }
                let _ = ui.button("Settings");
                let _ = ui.button("Help");
            });
        });

        // Machine list: one row per definition under `config_dir()/machines`.
        // `resizable` gives the draggable divider between the list and the
        // detail/photo pane.
        egui::SidePanel::left("manager_machine_list")
            .resizable(true)
            .default_width(LIST_DEFAULT_WIDTH)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show(ctx, |ui| {
                self.draw_machine_list(ui);
            });

        // Right pane: the selected machine's detail/edit form, or — with
        // nothing selected — a random photo asset, centered and scaled to
        // fit.
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(index) = self.selected {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.draw_detail(ui, index);
                });
            } else if let Some(texture) = &self.photo_texture {
                ui.centered_and_justified(|ui| {
                    ui.add(
                        egui::Image::new(texture)
                            .max_size(ui.available_size())
                            .maintain_aspect_ratio(true),
                    );
                });
            }
        });

        self.draw_delete_confirmation(ctx);
        self.refresh_due_thumbnails();
        self.draw_running_vms(ctx);
    }
}

/// Open the manager as the application's main window (blocks until close,
/// like `eframe::run_native` everywhere else).
pub fn run() -> eframe::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_icon(icon)
            .with_title("CocoVM"),
        ..Default::default()
    };
    let machines_dir = machine_def::machines_dir();
    // A machine definition that can't be read or doesn't validate is fatal:
    // exit with the reason rather than open a manager with a silently
    // wrong machine list (user decision 2026-07-19).
    let entries: Vec<MachineEntry> = match machines_dir.as_deref() {
        Some(dir) => match machine_def::load_all(dir) {
            Ok(defs) => defs
                .into_iter()
                .map(|(slug, def)| MachineEntry::new(slug, def))
                .collect(),
            Err(e) => {
                eprintln!("coco: cannot load machine definitions: {e}");
                std::process::exit(1);
            }
        },
        None => Vec::new(),
    };
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(move |cc| {
            crate::log_renderer_info(cc);
            Ok(Box::new(ManagerApp::new(
                photo_view::random(),
                machines_dir,
                machine_def::artifacts_root(),
                entries,
            )))
        }),
    )
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod tests;
