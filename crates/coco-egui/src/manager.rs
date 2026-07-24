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

use coco_core::MachineConfig;
use eframe::egui;

use crate::photo_view::{self, Photo};
use crate::{machine_def, new_vm, CocoApp};

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

/// List-row / detail-pane status labels. Never persisted
/// (`plan-machine-persistence.md` "Decisions" — "Runtime status … is never
/// persisted"): purely a function of [`MachineEntry::vm`] at draw time, see
/// [`vm_status_label`].
const STATUS_RUNNING: &str = "Running";
const STATUS_PAUSED: &str = "Paused";
const STATUS_STOPPED: &str = "Stopped";

/// Window size of a launched VM's own native OS window: the same formula
/// `main()` uses for the direct-boot window (`main.rs`'s `SCALE`/
/// `TARGET_ASPECT`/`MENU_BAR_H`/`TOOLBAR_H`/`STATUS_BAR_H`), sized for the
/// aspect-corrected (wider) image so it always fits.
fn vm_window_inner_size() -> egui::Vec2 {
    let img_h = coco_core::video::FB_H as f32 * crate::SCALE;
    let win_w = img_h * crate::TARGET_ASPECT;
    let win_h = img_h + crate::MENU_BAR_H + crate::TOOLBAR_H + crate::STATUS_BAR_H;
    egui::vec2(win_w, win_h)
}

/// Default size of the `ViewportClass::Embedded` fallback's `egui::Window`
/// (`draw_running_vms`) — deliberately much smaller than
/// [`vm_window_inner_size`]'s full native-window formula. That size (which
/// includes room for a menu bar/toolbar/status bar this fallback never
/// draws) is often close to or larger than the *entire* embedded canvas, so
/// even anchored to a corner it can span most of the screen and silently
/// eat clicks meant for the manager's own panels underneath (topmost window
/// wins pointer routing at a given position). A small preview loses nothing
/// real: this fallback only ever shows the bare display, never chrome.
const EMBEDDED_FALLBACK_SIZE: egui::Vec2 = egui::vec2(320.0, 240.0);

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
/// Fat transport-button geometry: minimum button size and glyph point size.
const TRANSPORT_BUTTON_SIZE: egui::Vec2 = egui::vec2(56.0, 40.0);
const TRANSPORT_GLYPH_SIZE: f32 = 24.0;
/// Gap separating the transport pair from the console-style buttons
/// (Suspend/Reset) and the status label.
const TRANSPORT_GROUP_GAP: f32 = 12.0;

/// One fat transport button ([`TRANSPORT_BUTTON_SIZE`]).
fn transport_button(ui: &mut egui::Ui, glyph: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(glyph).size(TRANSPORT_GLYPH_SIZE))
            .min_size(TRANSPORT_BUTTON_SIZE),
    )
}

/// Error text for Create/Save when [`ManagerApp::machines_dir`] is `None`
/// (no home directory — `paths::config_dir` docs).
const NO_CONFIG_DIR: &str = "no config directory available";

/// Default display name seeded into the "New…" dialog's Name field —
/// [`MachineConfig::default`]'s model, the same default the bare-invocation
/// direct-boot path (`main.rs`) and the dialog's own draft start from.
fn default_new_name() -> String {
    crate::machine_label(MachineConfig::default().variant).to_string()
}

/// One machine-list entry: a slug (file stem, also the identity used for
/// save/rename bookkeeping — `machine_def.rs` "Identity = slug") plus its
/// parsed definition, or the error from a failed parse/validate
/// (`machine_def::load_all`). Kept as a `Result` rather than dropping bad
/// files so the row can show an error badge instead of hiding the machine.
pub struct MachineEntry {
    pub slug: String,
    pub def: Result<machine_def::MachineDef, String>,
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
}

impl MachineEntry {
    /// `slug` + a freshly loaded/created `def`, with no VM running yet and
    /// no stale launch error — the state every entry starts in, whether
    /// loaded from disk ([`ManagerApp`]'s `run`) or just created (`Self`'s
    /// callers previously wrote out the `vm`/`launch_error` fields by hand;
    /// this constructor is what keeps that from drifting as more per-entry
    /// runtime state gets added later).
    pub(crate) fn new(slug: String, def: Result<machine_def::MachineDef, String>) -> Self {
        Self {
            slug,
            def,
            vm: None,
            launch_error: None,
            thumbnail: None,
            thumbnail_load_attempted: false,
            last_thumbnail_write: None,
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

/// Height the row's own text column (name, subtitle, status — three
/// `TextStyle::Body`-sized lines with `ui.vertical`'s default item spacing
/// between them) will render at, used to size the thumbnail to reach the
/// same bottom edge as the status line (user follow-up to step 6: "should
/// use the height available... go to the same edge as Stopped"). Computed
/// from text metrics up front rather than measured after layout, since the
/// thumbnail is the *first* widget placed in the row's `horizontal` — by
/// the time the text column's actual rendered height is known, the
/// thumbnail's own space is already allocated. All three lines use the
/// default `Body` text style at its default size (`.strong()`/`ui.weak()`
/// only change weight/color, not size), so one line height covers all
/// three, and `ui.vertical`'s gaps are exactly `ui.spacing().item_spacing.y`
/// — reproducing both here needs no second/probing layout pass.
fn row_content_height(ui: &egui::Ui) -> f32 {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let line_height = ui.fonts_mut(|f| f.row_height(&font_id));
    let spacing = ui.spacing().item_spacing.y;
    line_height * 3.0 + spacing * 2.0
}

/// One list row's thumbnail: the resolved preview `texture` — a live VM's
/// framebuffer, or a stopped machine's saved [`THUMBNAIL_FILE`]; the caller
/// resolves that priority — sized to `height` tall (see
/// [`row_content_height`]) at the fixed [`THUMBNAIL_ASPECT`], the same 4:3
/// the emulator's own display corrects to (framebuffer pixels aren't
/// square, so the texture's raw aspect would stretch the picture). It's one
/// extra quad reusing an already-uploaded texture, not an extra upload
/// (`docs/plan-machine-persistence.md` step 6, "Running/paused VM" bullet).
/// A paused VM's texture simply stops changing, so the thumbnail freezes on
/// its last frame with no special casing needed. With no texture — a
/// stopped machine, or a VM whose first frame hasn't uploaded one yet —
/// just the placeholder fill shows. Allocates its own space and returns the
/// rect it claimed.
fn draw_row_thumbnail(
    ui: &mut egui::Ui,
    height: f32,
    texture: Option<&egui::TextureHandle>,
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(height * THUMBNAIL_ASPECT, height),
        egui::Sense::hover(),
    );

    let painter = ui.painter();
    painter.rect_filled(rect, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_PLACEHOLDER_FILL);
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    rect
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

/// The detail pane's working copy of the selected entry's definition
/// (`plan-machine-persistence.md` step 4). `saved` is the last-loaded (or
/// last-saved) snapshot, compared against `def` for the Save button's dirty
/// indicator and restored by Revert.
struct EditDraft {
    /// Which entry this draft belongs to — a mismatch (a different row was
    /// clicked) means the draft must be reseeded before it's shown again.
    slug: String,
    saved: machine_def::MachineDef,
    def: machine_def::MachineDef,
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
    /// The right pane's edit draft for `selected`, when it's an `Ok` entry.
    /// `None` while nothing is selected, or the selected entry is an `Err`
    /// (nothing to edit).
    edit: Option<EditDraft>,
    /// Message from the last failed Save, shown under the Save/Revert row
    /// until the next attempt or a fresh selection.
    save_error: Option<String>,
    /// The "New…" dialog, reused from the direct-boot flow
    /// (`new_vm::NewVmDialog::new_for_manager` turns its Name row on).
    new_vm: new_vm::NewVmDialog,
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
            save_error: None,
            new_vm: new_vm::NewVmDialog::new_for_manager(),
        }
    }

    /// "New…": open the dialog seeded with a default draft and display name.
    fn open_new_dialog(&mut self) {
        self.new_vm.open_new(MachineConfig::default(), default_new_name());
    }

    /// "Create" in the "New…" dialog: build a definition from the draft
    /// config and the Name field, uniquify its slug against the current
    /// list, save it, and select the new row. Does NOT boot anything
    /// (`plan-machine-persistence.md` step 3). Save failures are reported in
    /// the dialog's own error field so it stays open for another try, the
    /// same contract `CocoApp::create_vm` follows for the direct-boot path.
    fn create_machine(&mut self, config: MachineConfig) {
        let Some(dir) = self.machines_dir.clone() else {
            self.new_vm.error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let name = self.new_vm.name.trim();
        let name = if name.is_empty() { default_new_name() } else { name.to_string() };

        let base = machine_def::slugify(&name);
        // Check both the in-memory list (loaded once at startup) and the
        // directory itself: `entries` misses any `<slug>.toml` written by a
        // second running instance, hand-edited in a terminal since startup
        // (an explicit design goal — `machine_def.rs` module doc), or
        // present on disk but absent from `entries` because `load_all`
        // swallowed a transient `read_dir` error. Without the on-disk check,
        // `machine_def::save`'s unconditional rename would silently
        // overwrite that file.
        let taken = |candidate: &str| {
            self.entries.iter().any(|e| e.slug == candidate)
                || dir.join(format!("{candidate}.toml")).exists()
        };
        let slug = machine_def::unique_slug(&base, &taken);

        let created = Some(chrono::Local::now().format(machine_def::DATE_FORMAT).to_string());
        let def = machine_def::MachineDef::from_config(name, created, &config);

        match machine_def::save(&dir, &slug, &def) {
            Ok(()) => {
                let index = self.entries.partition_point(|e| e.slug < slug);
                self.entries.insert(index, MachineEntry::new(slug, Ok(def)));
                self.selected = Some(index);
                self.edit = None; // reseeded from the new entry when the detail pane next draws
                self.save_error = None;
                self.new_vm.close();
            }
            Err(e) => self.new_vm.error = Some(e),
        }
    }

    /// Left panel: the machine list. `ui.set_min_width` (rather than only
    /// `take_available_space` on the empty case) keeps the `SidePanel`'s
    /// divider draggable in both states — an empty ui claims no space,
    /// which disables the resize drag (`SidePanel::resizable` docs).
    fn draw_machine_list(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        if self.entries.is_empty() {
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for i in 0..self.entries.len() {
                self.draw_machine_row(ui, i);
            }
        });
    }

    /// One machine-list row: placeholder thumbnail + name/subtitle/status
    /// for an `Ok` entry, or the file stem + an error badge for an `Err`
    /// one. Clicking anywhere in the row selects it (`ui.interact` over the
    /// frame's rect — the row's own labels aren't themselves interactive).
    fn draw_machine_row(&mut self, ui: &mut egui::Ui, i: usize) {
        // A stopped machine's saved preview, if any, loads (once) before the
        // row draws so this frame can already show it.
        self.ensure_row_thumbnail(&ui.ctx().clone(), i);
        let selected = self.selected == Some(i);
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame_rect = egui::Frame::new()
            .fill(fill)
            .inner_margin(ROW_MARGIN)
            .corner_radius(ROW_CORNER_RADIUS)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    let content_height = row_content_height(ui);
                    // Preview priority: a live VM's framebuffer texture,
                    // else the saved thumbnail.png loaded above, else the
                    // bare placeholder fill.
                    let texture = self.entries[i]
                        .vm
                        .as_deref()
                        .and_then(CocoApp::framebuffer_texture)
                        .or(self.entries[i].thumbnail.as_ref());
                    draw_row_thumbnail(ui, content_height, texture);

                    match &self.entries[i].def {
                        Ok(def) => {
                            let config = def
                                .to_machine_config()
                                .expect("list entries are validated on load/save");
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&def.name).strong());
                                ui.label(format!(
                                    "{} · {}",
                                    crate::machine_label(config.variant),
                                    new_vm::ram_label(config.memory),
                                ));
                                ui.weak(vm_status_label(&self.entries[i]));
                            });
                        }
                        Err(err) => {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(&self.entries[i].slug).strong());
                                ui.label(
                                    egui::RichText::new(format!("⚠ {err}"))
                                        .color(ui.visuals().error_fg_color)
                                        .small(),
                                )
                                .on_hover_text(err.as_str());
                            });
                        }
                    }
                });
            })
            .response
            .rect;

        let click_id = ui.id().with(("machine_row", i));
        if ui.interact(frame_rect, click_id, egui::Sense::click()).clicked() {
            self.selected = Some(i);
            self.save_error = None;
        }
    }

    /// Right pane for the selected entry: the edit form for an `Ok`
    /// definition, or the error + file path for an `Err` one.
    fn draw_detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let slug = self.entries[index].slug.clone();
        match self.entries[index].def.clone() {
            Ok(def) => self.draw_detail_ok(ui, index, slug, def),
            Err(err) => {
                self.edit = None;
                ui.heading(&slug);
                ui.colored_label(ui.visuals().error_fg_color, &err);
                if let Some(dir) = &self.machines_dir {
                    ui.monospace(dir.join(format!("{slug}.toml")).display().to_string());
                }
            }
        }
    }

    /// The editable form for a successfully-parsed definition. Split out of
    /// [`Self::draw_detail`] so the `Ok` branch can freely borrow the rest of
    /// `self` (`self.machines_dir`, `self.entries`) while `self.edit` is
    /// temporarily taken out — an `&mut EditDraft` alongside those would
    /// otherwise conflict with the single `&mut self` this method needs.
    fn draw_detail_ok(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        slug: String,
        def: machine_def::MachineDef,
    ) {
        if self.edit.as_ref().is_none_or(|e| e.slug != slug) {
            // Seed `saved` normalized the same way the per-frame hardware
            // repack below normalizes `def` (`HardwareDto::from_config`
            // always writes an explicit `vdg` — see `normalize_hardware`'s
            // doc). Otherwise a file that legally omits `vdg` starts the
            // draft already dirty from mere selection, and Revert could
            // never clear it: it would restore `saved`'s `None`, but the
            // very next frame's repack re-normalizes `def` back to `Some`.
            let saved = normalize_hardware(def);
            self.edit = Some(EditDraft { slug: slug.clone(), saved: saved.clone(), def: saved });
            self.save_error = None;
        }
        let mut edit = self.edit.take().expect("just ensured above");

        ui.add(egui::TextEdit::singleline(&mut edit.def.name).font(egui::TextStyle::Heading));
        ui.add_space(DETAIL_SECTION_GAP);

        // Run controls (see the transport-glyph constants' doc for the
        // split-metaphor rationale): fat deck-style transport for execution,
        // ordinary buttons for Suspend (a placeholder until save-states
        // land) and the console Reset. `is_running` is copied out before the
        // buttons so the click handlers below can freely call `&mut self`
        // methods (`start_vm`/`stop_vm`/`toggle_running`) without fighting a
        // borrow of `self.entries[index].vm` still held by a `match` on it.
        let is_running = self.entries[index].vm.as_ref().map(|vm| vm.is_running());
        ui.horizontal(|ui| {
            // One Play/Pause toggle: ▶ starts a stopped machine or resumes
            // a paused one; ⏸ pauses a running one.
            let (glyph, hover) = match is_running {
                None => (PLAY_GLYPH, "Start the machine"),
                Some(true) => (
                    PAUSE_GLYPH,
                    "Pause emulation — freeze the machine in place; resume anytime. \
                     Not saved: pausing does not survive quitting the manager.",
                ),
                Some(false) => (PLAY_GLYPH, "Resume emulation"),
            };
            if transport_button(ui, glyph, true).on_hover_text(hover).clicked() {
                match is_running {
                    None => self.start_vm(index),
                    Some(_) => {
                        if let Some(vm) = self.entries[index].vm.as_mut() {
                            vm.toggle_running();
                        }
                    }
                }
            }
            if transport_button(ui, STOP_GLYPH, is_running.is_some())
                .on_hover_text(
                    "Shut down the machine — like flipping the power switch; \
                     unsaved work inside it is lost",
                )
                .clicked()
            {
                self.stop_vm(index);
            }

            ui.add_space(TRANSPORT_GROUP_GAP);
            // Suspend is the *heavy* freeze — dump the whole machine to disk
            // and resume much later, even on another computer. It ships with
            // the save-states milestone (`docs/plan-save-states.md`); the
            // disabled button teaches the model before the feature exists.
            let _ = ui
                .add_enabled(false, egui::Button::new("Suspend"))
                .on_disabled_hover_text(
                    "Freeze this machine to disk and free it — resume later, even after \
                     quitting or on another computer. Coming with save-states.",
                );
            if ui
                .add_enabled(is_running.is_some(), egui::Button::new("Reset"))
                .on_hover_text("Press the machine's reset button — the machine stays on")
                .clicked()
                && let Some(vm) = self.entries[index].vm.as_mut()
            {
                vm.machine.reset();
            }

            ui.add_space(TRANSPORT_GROUP_GAP);
            ui.label(egui::RichText::new(vm_status_label(&self.entries[index])).strong());
        });
        if edit.def != edit.saved {
            ui.small("Unsaved changes won't apply until this machine is saved.");
        }
        if let Some(err) = &self.entries[index].launch_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);

        // Hardware form: shared with the "New…" dialog (`new_vm.rs`'s
        // `config_form_rows`) so the RAM/VDG/PAL constraint rules live in
        // exactly one place. It edits a bare `MachineConfig`, so the DTO is
        // unpacked into one before the grid and repacked after — `rom` (not
        // part of `MachineConfig`) passes through untouched; there's no
        // editor for it yet.
        let mut config = edit.def.to_machine_config().expect("Ok entries validate on load/save");
        egui::Grid::new(("detail_hw_grid", slug.clone()))
            .num_columns(2)
            .spacing(new_vm::FORM_GRID_SPACING)
            .show(ui, |ui| {
                new_vm::config_form_rows(ui, &format!("detail-{slug}"), &mut config);
            });
        edit.def.hardware = machine_def::HardwareDTO::from_config(&config, edit.def.hardware.rom.clone());

        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(egui::RichText::new("Media").strong());
        ui.small("Read-only for now — attaching/detaching media lands in a later step.");
        for (label, value) in media_rows(&edit.def.media) {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.monospace(value);
            });
        }

        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(egui::RichText::new("Peripherals").strong());
        ui.checkbox(&mut edit.def.peripherals.mpi, "MultiPak Interface");
        ui.checkbox(&mut edit.def.peripherals.rtc, "Disto RTC");

        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(egui::RichText::new("UI").strong());
        ui.checkbox(&mut edit.def.ui.aspect_correct, "4:3 aspect correction");
        ui.horizontal(|ui| {
            ui.label("Keyboard mode:");
            ui.radio_value(&mut edit.def.ui.kb_mode, machine_def::KbModeDTO::Positional, "Positional");
            ui.radio_value(&mut edit.def.ui.kb_mode, machine_def::KbModeDTO::Symbolic, "Symbolic");
        });

        ui.add_space(DETAIL_SECTION_GAP);
        let dirty = edit.def != edit.saved;
        ui.horizontal(|ui| {
            let save_label = if dirty { "Save*" } else { "Save" };
            if ui.add_enabled(dirty, egui::Button::new(save_label)).clicked() {
                self.save_draft(index, &mut edit);
            }
            if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                edit.def = edit.saved.clone();
                self.save_error = None;
            }
        });
        if let Some(err) = &self.save_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        self.edit = Some(edit);
    }

    /// Write `edit.def` to its TOML file (rename-migration is out of scope —
    /// renaming only ever changes `[name]`, never the slug/file, per
    /// `plan-machine-persistence.md` "Identity = slug") and, on success,
    /// mark the draft clean and refresh the list-row entry it belongs to.
    fn save_draft(&mut self, index: usize, edit: &mut EditDraft) {
        let Some(dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        match machine_def::save(&dir, &edit.slug, &edit.def) {
            Ok(()) => {
                edit.saved = edit.def.clone();
                self.entries[index].def = Ok(edit.def.clone());
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Detail pane's Start button: launch `entries[index]`'s *saved*
    /// definition (`crate::launch_machine`) — not the in-progress edit
    /// draft, which may hold changes the user hasn't saved yet (the small
    /// note next to the button in [`Self::draw_detail_ok`] is the only
    /// warning about that). A stopped entry always has `vm: None`, so this
    /// only ever replaces `None` with `Some`; an entry that's already
    /// running has no Start button to click (see the `is_running` match in
    /// `draw_detail_ok`).
    fn start_vm(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.launch_error = None;
        let Ok(def) = &entry.def else {
            // Unreachable via the UI (an `Err` entry's detail pane has no
            // Start button), kept as a guard rather than a panic in case a
            // future caller reaches this some other way.
            return;
        };
        match crate::launch_machine(def, &entry.slug) {
            Ok(vm) => entry.vm = Some(Box::new(vm)),
            Err(e) => entry.launch_error = Some(e),
        }
    }

    /// Stop button (and the VM window's own close box, via
    /// [`Self::draw_running_vms`]): flush dirty disks/tape back to their
    /// files — the same exit contract `CocoApp::on_exit` runs for the
    /// direct-boot window — then drop the VM, returning the row to Stopped.
    fn stop_vm(&mut self, index: usize) {
        self.write_entry_thumbnail(index);
        if let Some(mut vm) = self.entries[index].vm.take() {
            vm.flush_media();
        }
    }

    /// Snapshot `entries[index]`'s running VM screen into its artifact dir
    /// (see [`write_thumbnail_png`]) and invalidate the row's cached preview
    /// texture so the next draw reloads the fresh file. No-op for a stopped
    /// entry or when no artifact root exists. Capture happens between update
    /// frames, so the framebuffer always holds a whole rendered field —
    /// never a torn, mid-render frame.
    fn write_entry_thumbnail(&mut self, index: usize) {
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let entry = &mut self.entries[index];
        let Some(vm) = entry.vm.as_ref() else {
            return;
        };
        let (w, h) = (vm.machine.fb_width, vm.machine.fb_height);
        if let Err(e) = write_thumbnail_png(&root.join(&entry.slug), &vm.machine.framebuffer, w, h)
        {
            tracing::warn!("thumbnail for '{}': {e}", entry.slug);
        }
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
        entry.last_thumbnail_write = Some(Instant::now());
    }

    /// [`THUMBNAIL_REFRESH`] cadence for every running VM — called once per
    /// `update()`. The first write happens right after Start
    /// (`last_thumbnail_write` starts `None`), so even a young machine has
    /// an on-disk preview if the process dies.
    fn refresh_due_thumbnails(&mut self) {
        if self.artifacts_root.is_none() {
            return;
        }
        for i in 0..self.entries.len() {
            if self.entries[i].vm.is_none() {
                continue;
            }
            let due = self.entries[i]
                .last_thumbnail_write
                .is_none_or(|last| last.elapsed() >= THUMBNAIL_REFRESH);
            if due {
                self.write_entry_thumbnail(i);
            }
        }
    }

    /// Lazily load a stopped entry's saved [`THUMBNAIL_FILE`] into a texture
    /// the first time its row draws (and again after
    /// [`Self::write_entry_thumbnail`] invalidates the cache). Failures just
    /// leave the placeholder — the preview is a cache, never required state.
    fn ensure_row_thumbnail(&mut self, ctx: &egui::Context, index: usize) {
        let entry = &mut self.entries[index];
        if entry.vm.is_some() || entry.thumbnail_load_attempted {
            return;
        }
        entry.thumbnail_load_attempted = true;
        let Some(root) = &self.artifacts_root else {
            return;
        };
        let Ok(image) = image::open(root.join(&entry.slug).join(THUMBNAIL_FILE)) else {
            return;
        };
        let image = image.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        entry.thumbnail = Some(ctx.load_texture(
            format!("thumbnail-{}", entry.slug),
            pixels,
            egui::TextureOptions::LINEAR,
        ));
    }

    /// One native OS window per running VM (`docs/plan-machine-persistence.md`
    /// "DECIDED: in-process, one native window per running VM"): an
    /// immediate viewport per entry with a VM, keyed by a stable id derived
    /// from the slug so egui reuses the same OS window across frames instead
    /// of respawning it (the same pattern `paper_view::PaperWindow::ui` uses
    /// for the printer-paper window). Called once per `ManagerApp::update`,
    /// after the manager's own panels.
    ///
    /// Close requests (the native window's close box, or the embedded
    /// fallback's `egui::Window` close button) are collected into a list and
    /// applied with [`Self::stop_vm`] after the loop — `stop_vm` needs
    /// `&mut self.entries[i]`, which would conflict with the `vm` this loop
    /// already holds taken out of that same slot for the duration of the
    /// viewport closure.
    fn draw_running_vms(&mut self, ctx: &egui::Context) {
        let mut to_stop: Vec<usize> = Vec::new();
        for i in 0..self.entries.len() {
            if self.entries[i].vm.is_none() {
                continue;
            }
            let slug = self.entries[i].slug.clone();
            let name = self.entries[i]
                .def
                .as_ref()
                .map(|d| d.name.clone())
                .unwrap_or_else(|_| slug.clone());
            let viewport_id = egui::ViewportId::from_hash_of(("vm-window", &slug));
            let inner_size = vm_window_inner_size();
            let builder = egui::ViewportBuilder::default()
                .with_title(name.clone())
                .with_inner_size(inner_size);

            // Taken out of the entry so the viewport closure below can hold
            // and mutate it without a conflicting borrow of `self` (the
            // closure also needs to push into `to_stop`, a local, not
            // `self` — so no `self` borrow is held across the closure at
            // all here).
            let mut vm = self.entries[i].vm.take().expect("checked Some above");
            let mut close_requested = false;
            ctx.show_viewport_immediate(viewport_id, builder, |child_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Degraded single-window fallback (kittest and other
                    // backends without native multi-window support, per
                    // `paper_view`'s module doc comment on the same
                    // pattern): don't draw `CocoApp`'s own menu bar/toolbar/
                    // status bar into the manager's shared `ctx` — that
                    // would interleave two independent sets of panels into
                    // one window. Show just the VM's display in a plain
                    // `egui::Window` instead; full chrome only exists as its
                    // own native OS window. The VM still runs:
                    // `step_emulation` is unconditional either way.
                    vm.step_emulation(child_ctx);
                    let mut open = true;
                    // Anchored, and capped at `EMBEDDED_FALLBACK_SIZE`
                    // rather than the native window's full
                    // `inner_size` (found the hard way, via a kittest
                    // regression: a window that large, even anchored to a
                    // corner, still spans most of a modest single-window
                    // canvas — e.g. the whole manager UI under kittest — and
                    // silently eats clicks meant for the manager's own
                    // panels underneath, since pointer routing goes to
                    // whichever window is topmost at that screen position.
                    // This fallback only ever shows the bare display anyway
                    // (no chrome), so a smaller preview loses nothing a
                    // real native window wouldn't already provide instead.
                    egui::Window::new(crate::window_title(child_ctx, &name))
                        .id(egui::Id::new(("vm-window-embedded", slug.as_str())))
                        .open(&mut open)
                        .resizable(false)
                        .default_size(EMBEDDED_FALLBACK_SIZE)
                        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -8.0))
                        .show(child_ctx, |ui| {
                            vm.draw_display(ui);
                        });
                    if !open {
                        close_requested = true;
                    }
                } else {
                    vm.window_ui(child_ctx);
                    if child_ctx.input(|i| i.viewport().close_requested()) {
                        close_requested = true;
                    }
                }
            });

            self.entries[i].vm = Some(vm);
            if close_requested {
                to_stop.push(i);
            }
        }
        for i in to_stop {
            self.stop_vm(i);
        }
    }
}

#[cfg(test)]
impl ManagerApp {
    /// The detail pane's current draft name, when one is shown — `ui_tests.rs`
    /// checks that selecting a row seeds the right draft without depending on
    /// how `egui::TextEdit` exposes its value to the accessibility tree.
    pub(crate) fn detail_name(&self) -> Option<&str> {
        self.edit.as_ref().map(|e| e.def.name.as_str())
    }
}

/// Resolve `def.hardware.vdg`'s per-variant default and write it back
/// explicitly, the same way `draw_detail_ok`'s per-frame hardware repack
/// does via `HardwareDto::from_config` (it always writes a concrete `vdg`,
/// unlike a hand-written file which may omit it — see `VdgDto`'s doc). Used
/// to seed `EditDraft::saved` so a definition that legally omits `vdg`
/// doesn't compare unequal to the draft `to_machine_config`/`from_config`
/// round-trip immediately produces. A definition that fails to validate
/// (shouldn't happen for an already-`Ok` list entry) is returned unchanged.
fn normalize_hardware(mut def: machine_def::MachineDef) -> machine_def::MachineDef {
    if let Ok(config) = def.to_machine_config() {
        def.hardware = machine_def::HardwareDTO::from_config(&config, def.hardware.rom.clone());
    }
    def
}

/// The `[media]` fields that are set, as `(row label, value)` pairs, in
/// schema-declaration order.
fn media_rows(media: &machine_def::MediaDTO) -> Vec<(&'static str, &str)> {
    [
        ("Cart", &media.cart),
        ("Disk 0", &media.disk0),
        ("Disk 1", &media.disk1),
        ("VHD 0", &media.vhd0),
        ("VHD 1", &media.vhd1),
        ("Tape", &media.tape),
    ]
    .into_iter()
    .filter_map(|(label, value)| value.as_deref().map(|v| (label, v)))
    .collect()
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

        // Toolbar: the manager actions. "Settings"/"Help" are still inert
        // scaffolding.
        egui::TopBottomPanel::top("manager_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("New…").clicked() {
                    self.open_new_dialog();
                }
                let _ = ui.button("Settings");
                let _ = ui.button("Help");
            });
        });

        if let new_vm::NewVmAction::Create(config) = self.new_vm.show(ctx) {
            self.create_machine(config);
        }

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
    let entries = machines_dir
        .as_deref()
        .map(|dir| {
            machine_def::load_all(dir)
                .into_iter()
                .map(|(slug, def)| MachineEntry::new(slug, def))
                .collect()
        })
        .unwrap_or_default();
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(move |_cc| {
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
mod tests {
    use super::*;
    use crate::machine_def::tests::TempDir;

    /// A tiny non-black RGBA frame (2×2, opaque red).
    const RED_FRAME: [u8; 16] = [
        0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF,
    ];
    /// Same geometry, uniformly black — the frame [`write_thumbnail_png`]'s
    /// blank-screen heuristic guards against.
    const BLACK_FRAME: [u8; 16] = [
        0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 0xFF,
    ];

    #[test]
    fn write_thumbnail_png_round_trips_and_leaves_no_tmp() {
        let dir = TempDir::new("thumb-roundtrip");
        write_thumbnail_png(dir.path(), &RED_FRAME, 2, 2).expect("write succeeds");

        assert!(!dir.path().join(format!("{THUMBNAIL_FILE}.tmp")).exists());
        let image = image::open(dir.path().join(THUMBNAIL_FILE)).expect("decodable PNG");
        assert_eq!((image.width(), image.height()), (2, 2));
    }

    #[test]
    fn uniformly_black_frame_keeps_the_previous_thumbnail() {
        let dir = TempDir::new("thumb-black-skip");
        write_thumbnail_png(dir.path(), &RED_FRAME, 2, 2).unwrap();
        let before = fs::read(dir.path().join(THUMBNAIL_FILE)).unwrap();

        write_thumbnail_png(dir.path(), &BLACK_FRAME, 2, 2).unwrap();
        let after = fs::read(dir.path().join(THUMBNAIL_FILE)).unwrap();
        assert_eq!(before, after, "a blank screen must not clobber a useful preview");
    }

    #[test]
    fn black_frame_is_still_written_when_no_previous_thumbnail_exists() {
        let dir = TempDir::new("thumb-black-first");
        write_thumbnail_png(dir.path(), &BLACK_FRAME, 2, 2).unwrap();
        assert!(dir.path().join(THUMBNAIL_FILE).exists());
    }
}
