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

/// File name of the auto-placed blank image the detail pane's
/// Disk N = Blank pick creates in the machine's artifact directory,
/// recorded in `[media].diskN` as a relative path.
fn blank_disk_file(drive: usize) -> String {
    format!("disk{drive}.dsk")
}

/// [`blank_disk_file`]'s cassette sibling, for `[media].tape`.
const BLANK_TAPE_FILE: &str = "tape.cas";

/// [`blank_disk_file`]'s VHD sibling, for `[media].vhdN`.
fn blank_vhd_file(drive: usize) -> String {
    format!("hd{drive}.vhd")
}

/// Display name (and slug source) of a freshly created machine
/// ([`ManagerApp::create_machine_now`]) — [`MachineConfig::default`]'s
/// model, the same default the bare-invocation direct-boot path (`main.rs`)
/// starts from.
fn default_new_name() -> String {
    crate::machine_label(MachineConfig::default().variant).to_string()
}

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

    /// "New…" (toolbar button and ⌘N): create a default machine *right
    /// now* — saved to disk under a uniquified slug, inserted in the list,
    /// and selected with the Name field focused — instead of opening a
    /// dialog. There is no Cancel; an unwanted machine is deleted like any
    /// other (context menu → Delete…). Does NOT boot anything.
    fn create_machine_now(&mut self) {
        let Some(dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let name = default_new_name();
        // Check both the in-memory list (loaded once at startup) and the
        // directory itself: `entries` misses any `<slug>.toml` written by a
        // second running instance or hand-placed since startup (an explicit
        // design goal — `machine_def.rs` module doc). Without the on-disk
        // check, `machine_def::save`'s unconditional rename would silently
        // overwrite that file.
        let taken = |candidate: &str| {
            self.entries.iter().any(|e| e.slug == candidate)
                || dir.join(format!("{candidate}.toml")).exists()
        };
        let slug = machine_def::unique_slug(&machine_def::slugify(&name), &taken);
        let created = Some(chrono::Local::now().format(machine_def::DATE_FORMAT).to_string());
        let def = machine_def::MachineDef::from_config(name, created, &MachineConfig::default());
        match machine_def::save(&dir, &slug, &def) {
            Ok(()) => {
                let index = self.entries.partition_point(|e| e.slug < slug);
                self.entries.insert(index, MachineEntry::new(slug, def));
                self.selected = Some(index);
                self.edit = None; // seeded from the new entry on next draw
                self.focus_name = true;
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Resolve one of the edit form's media picks to the string recorded in
    /// the definition's `[media]` section, creating the backing file for a
    /// Blank pick: auto-placed in `slug`'s artifact dir as `auto_file`
    /// (recorded relative — `machine_def::resolve_media_path`). Blank media
    /// is a 0-byte file — a blank 0-track JVC disk, an empty `.cas` tape or
    /// `.vhd`, the same starting point `CocoApp::{new_blank_disk, new_tape}`
    /// use; a leftover file under the same slug is reused rather than
    /// clobbered. The pick is rewritten to `File(recorded)` afterwards so
    /// the combo shows the placed file, not a stale "Blank".
    fn record_media_choice(
        &self,
        slug: &str,
        choice: &mut new_vm::MediaChoice,
        auto_file: String,
    ) -> Result<Option<String>, String> {
        let (path, recorded) = match &*choice {
            new_vm::MediaChoice::None => return Ok(None),
            new_vm::MediaChoice::File(path) => return Ok(Some(path.display().to_string())),
            new_vm::MediaChoice::Blank(Some(path)) => {
                (path.clone(), path.display().to_string())
            }
            new_vm::MediaChoice::Blank(None) => {
                let Some(root) = self.artifacts_root.clone() else {
                    return Err(NO_CONFIG_DIR.to_string());
                };
                let artifact_dir = root.join(slug);
                fs::create_dir_all(&artifact_dir)
                    .map_err(|e| format!("{}: {e}", artifact_dir.display()))?;
                (artifact_dir.join(&auto_file), auto_file)
            }
        };
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        *choice = new_vm::MediaChoice::File(PathBuf::from(&recorded));
        Ok(Some(recorded))
    }

    /// Pack the edit form back into a definition, starting from `base` (the
    /// entry's current definition) so everything the form doesn't edit —
    /// `name`, `created`, `[hardware].rom`, unknown keys — passes through
    /// untouched. Blank media picks create their backing files here (see
    /// [`Self::record_media_choice`]); this is the write moment, since with
    /// auto-save every change *is* a save. Errors (an unrepresentable form,
    /// a failed file creation) leave the definition unwritten and land in
    /// the pane's error label.
    fn pack_def(
        &self,
        base: &machine_def::MachineDef,
        slug: &str,
        form: &mut new_vm::MachineForm,
    ) -> Result<machine_def::MachineDef, String> {
        let mut def = base.clone();
        def.hardware =
            machine_def::HardwareDTO::from_config(&form.config, base.hardware.rom.clone());
        // The definition schema has no slot layout (yet): an FD-502 in an
        // MPI slot is recorded as fd502 = true, a slotted RTC as rtc = true,
        // and launch_machine re-seats them in their default slots.
        def.peripherals.fd502 = form.drives_available();
        def.peripherals.mpi = form.cartridge == new_vm::CartridgeChoice::MPI;
        def.peripherals.rtc = form.cartridge == new_vm::CartridgeChoice::RTC
            || form.mpi_slots.contains(&new_vm::SlotChoice::RTC);
        // A ROM Pak — in the port or slotted in the MPI — is recorded as
        // [media].cart. The schema holds a single pak and no slot layout
        // (launch_machine re-seats a slotted one in slot 0), so more than
        // one slotted pak cannot be represented.
        let mut slotted_paks = form.mpi_slots.iter().filter_map(|slot| match slot {
            new_vm::SlotChoice::RomPak(path) => Some(path),
            _ => None,
        });
        def.media.cart = match &form.cartridge {
            new_vm::CartridgeChoice::RomPak(path) => Some(path.display().to_string()),
            new_vm::CartridgeChoice::MPI => slotted_paks.next().map(|p| p.display().to_string()),
            _ => None,
        };
        if slotted_paks.next().is_some() {
            return Err(
                "a machine definition records a single ROM Pak — leave at most one slot \
                 with a pak"
                    .to_string(),
            );
        }
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.disks[drive], blank_disk_file(drive))?;
            match drive {
                0 => def.media.disk0 = recorded,
                _ => def.media.disk1 = recorded,
            }
        }
        def.media.tape =
            self.record_media_choice(slug, &mut form.tape, BLANK_TAPE_FILE.to_string())?;
        for drive in 0..crate::UI_DRIVES {
            let recorded =
                self.record_media_choice(slug, &mut form.vhds[drive], blank_vhd_file(drive))?;
            match drive {
                0 => def.media.vhd0 = recorded,
                _ => def.media.vhd1 = recorded,
            }
        }
        def.ui.aspect_correct = form.aspect_correct;
        def.ui.kb_mode = match form.kb_mode {
            crate::KbMode::Positional => machine_def::KbModeDTO::Positional,
            crate::KbMode::Symbolic => machine_def::KbModeDTO::Symbolic,
        };
        Ok(def)
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

                    let def = &self.entries[i].def;
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
                });
            })
            .response
            .rect;

        let click_id = ui.id().with(("machine_row", i));
        let response = ui.interact(frame_rect, click_id, egui::Sense::click());
        if response.clicked() {
            self.selected = Some(i);
            self.save_error = None;
        }
        // Per-row context menu. Its items act on the row under the cursor
        // (this `i`), never on `self.selected` — right-click deliberately
        // does not move the selection cue (user decision 2026-07-23); only
        // "Show config" moves it, because showing the detail pane *is*
        // selecting.
        response.context_menu(|ui| {
            let has_vm = self.entries[i].vm.is_some();
            if ui.add_enabled(!has_vm, egui::Button::new("Start")).clicked() {
                self.start_vm(i);
                ui.close();
            }
            let _ = ui
                .add_enabled(false, egui::Button::new("Suspend"))
                .on_disabled_hover_text(SUSPEND_DISABLED_HOVER);
            if ui.add_enabled(has_vm, egui::Button::new("Reset")).clicked() {
                if let Some(vm) = self.entries[i].vm.as_mut() {
                    vm.machine.reset();
                }
                ui.close();
            }
            if ui.add_enabled(has_vm, egui::Button::new("Stop")).clicked() {
                self.stop_vm(i);
                ui.close();
            }
            ui.separator();
            if ui.button("Show config").clicked() {
                self.selected = Some(i);
                self.save_error = None;
                ui.close();
            }
            ui.separator();
            if ui.button("Delete…").clicked() {
                self.pending_delete = Some(self.entries[i].slug.clone());
                ui.close();
            }
        });
    }

    /// Right pane for the selected entry: its edit form.
    fn draw_detail(&mut self, ui: &mut egui::Ui, index: usize) {
        let slug = self.entries[index].slug.clone();
        let def = self.entries[index].def.clone();
        self.draw_detail_ok(ui, index, slug, def);
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
            let mut form = seed_form(&def);
            // The auto-save baseline is the seeded form's own repack — see
            // `EditState::packed`'s doc for why it must not be `def`
            // itself. A freshly seeded form holds no Blank picks and at
            // most one pak, so this pack can't fail or touch a file.
            let packed = self
                .pack_def(&def, &slug, &mut form)
                .expect("a seeded form always packs");
            self.edit = Some(EditState {
                slug: slug.clone(),
                name: def.name.clone(),
                form,
                packed,
            });
            self.save_error = None;
        }
        let mut edit = self.edit.take().expect("just ensured above");

        // The Name field commits on focus loss/Enter — not per keystroke,
        // so "C", "Co", "CoC"… aren't each saved (and re-slugified) on the
        // way to the real name. Everything else in the pane saves on change.
        let name_response =
            ui.add(egui::TextEdit::singleline(&mut edit.name).font(egui::TextStyle::Heading));
        if self.focus_name {
            name_response.request_focus();
            self.focus_name = false;
        }
        if name_response.lost_focus() {
            self.commit_name(index, &mut edit);
        }
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
            let _ = ui
                .add_enabled(false, egui::Button::new("Suspend"))
                .on_disabled_hover_text(SUSPEND_DISABLED_HOVER);
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
        if let Some(err) = &self.entries[index].launch_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);

        // The shared machine form — the exact rows the "New…" dialog draws
        // (`new_vm::MachineForm`), hosted in the pane's own grid.
        egui::Grid::new(("detail_form", slug.clone()))
            .num_columns(2)
            .spacing(new_vm::FORM_GRID_SPACING)
            .show(ui, |ui| {
                edit.form.rows(ui);
            });
        if self.entries[index].vm.is_some() {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.small("Changes apply the next time this machine starts.");
        }

        // Auto-save: every change writes straight back to the definition
        // file (no Save/Revert — user decision 2026-07-24; the write is
        // atomic, `machine_def::save`). On a persistent failure this
        // retries every frame — harmless for a tiny file, and it keeps the
        // error label current.
        match self.pack_def(&self.entries[index].def, &slug, &mut edit.form) {
            Ok(new_def) => {
                if new_def != edit.packed {
                    let result = match self.machines_dir.clone() {
                        Some(dir) => machine_def::save(&dir, &slug, &new_def),
                        None => Err(NO_CONFIG_DIR.to_string()),
                    };
                    match result {
                        Ok(()) => {
                            self.entries[index].def = new_def.clone();
                            edit.packed = new_def;
                            self.save_error = None;
                        }
                        Err(e) => self.save_error = Some(e),
                    }
                }
            }
            Err(e) => self.save_error = Some(e),
        }
        if let Some(err) = &self.save_error {
            ui.add_space(DETAIL_SECTION_GAP);
            ui.colored_label(ui.visuals().error_fg_color, err);
        }

        self.edit = Some(edit);
    }

    /// The Name field committed ([`Self::draw_detail_ok`] — focus left it):
    /// an empty draft reverts to the saved name; a change saves immediately
    /// under the *current* slug, then the file/artifact names follow the
    /// new name via [`Self::migrate_slug`] — deferred to
    /// [`Self::apply_pending_renames`] (next frame, or after Stop for a
    /// running machine).
    fn commit_name(&mut self, index: usize, edit: &mut EditState) {
        let trimmed = edit.name.trim().to_string();
        if trimmed.is_empty() || trimmed == self.entries[index].def.name {
            edit.name = self.entries[index].def.name.clone();
            return;
        }
        self.entries[index].def.name = trimmed.clone();
        edit.name = trimmed;
        // Keep the auto-save baseline in step: the name isn't one of the
        // form's fields, and a stale `packed.name` would make the next
        // repack look changed and re-save redundantly.
        edit.packed.name = self.entries[index].def.name.clone();
        let result = match self.machines_dir.clone() {
            Some(dir) => machine_def::save(&dir, &edit.slug, &self.entries[index].def),
            None => Err(NO_CONFIG_DIR.to_string()),
        };
        match result {
            Ok(()) => {
                self.save_error = None;
                self.entries[index].rename_pending = true;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Rename `entries[index]`'s `<slug>.toml` and artifact directory to
    /// match its (already saved) display name. The slug is the identity
    /// (`machine_def.rs` "Identity = slug") and nothing else persists it —
    /// relative `[media]` entries name files *inside* the artifact dir —
    /// so a rename is exactly these two filesystem moves, uniquified like
    /// create. Only safe with the VM stopped (a running VM writes
    /// `thumbnail.png` into the artifact dir by path); callers guard on
    /// that. The list is re-sorted afterwards, with `selected`, the edit
    /// state, and a pending delete all following their entry.
    fn migrate_slug(&mut self, index: usize) {
        self.entries[index].rename_pending = false;
        let Some(dir) = self.machines_dir.clone() else {
            return;
        };
        let old = self.entries[index].slug.clone();
        let base = machine_def::slugify(&self.entries[index].def.name);
        let slugs: Vec<String> = self.entries.iter().map(|e| e.slug.clone()).collect();
        let taken = |candidate: &str| {
            candidate != old
                && (slugs.iter().any(|s| s == candidate)
                    || dir.join(format!("{candidate}.toml")).exists())
        };
        let new = machine_def::unique_slug(&base, &taken);
        if new == old {
            return;
        }
        let old_path = dir.join(format!("{old}.toml"));
        let new_path = dir.join(format!("{new}.toml"));
        if let Err(e) = fs::rename(&old_path, &new_path) {
            self.save_error = Some(format!("{}: {e}", old_path.display()));
            return;
        }
        if let Some(root) = &self.artifacts_root {
            let old_dir = root.join(&old);
            if old_dir.exists()
                && let Err(e) = fs::rename(&old_dir, root.join(&new))
            {
                // Roll the definition back under the old slug: a stale slug
                // beats relative [media] entries resolving into a directory
                // that no longer matches the definition's file name.
                let _ = fs::rename(&new_path, &old_path);
                self.save_error = Some(format!("{}: {e}", old_dir.display()));
                return;
            }
        }
        self.entries[index].slug = new.clone();
        // Keep the list alphabetical and every slug-keyed pointer valid.
        let selected_slug = self.selected.map(|s| self.entries[s].slug.clone());
        let entry = self.entries.remove(index);
        let at = self.entries.partition_point(|e| e.slug < entry.slug);
        self.entries.insert(at, entry);
        if let Some(slug) = selected_slug {
            self.selected = self.entries.iter().position(|e| e.slug == slug);
        }
        if let Some(edit) = self.edit.as_mut()
            && edit.slug == old
        {
            edit.slug = new.clone();
        }
        if self.pending_delete.as_deref() == Some(old.as_str()) {
            self.pending_delete = Some(new);
        }
    }

    /// Run once per `update()`, before any panel draws (so row indices stay
    /// stable for the whole frame): migrate the slug of every renamed
    /// machine whose VM is gone. Several can be pending at once (rename a
    /// running machine, select another, rename it too…), and each
    /// [`Self::migrate_slug`] re-sorts the list — hence re-`position` from
    /// scratch per iteration rather than iterating indices. Terminates
    /// because `migrate_slug` clears `rename_pending` unconditionally,
    /// success or failure.
    fn apply_pending_renames(&mut self) {
        while let Some(index) = self
            .entries
            .iter()
            .position(|e| e.rename_pending && e.vm.is_none())
        {
            self.migrate_slug(index);
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
        match crate::launch_machine(&entry.def, &entry.slug) {
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

    /// The confirmation modal behind the context menu's "Delete…"
    /// ([`ManagerApp::pending_delete`]), drawn once per `update()`. Esc,
    /// Cancel, and a click outside all dismiss without deleting; the confirm
    /// button reads "Stop and Delete" when the machine is running, since
    /// deleting stops it first. A failed delete reports its error inside the
    /// modal and leaves it open.
    fn draw_delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(slug) = self.pending_delete.clone() else {
            return;
        };
        let Some(index) = self.entries.iter().position(|e| e.slug == slug) else {
            // The row vanished under the pending confirmation (see
            // `pending_delete`'s doc) — nothing left to delete.
            self.pending_delete = None;
            return;
        };
        let running = self.entries[index].vm.is_some();
        let name = self.entries[index].def.name.clone();
        let mut dismissed = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete_machine")).show(ctx, |ui| {
            ui.heading(format!("Delete “{name}”?"));
            ui.add_space(DETAIL_SECTION_GAP);
            ui.label(
                "The machine's definition is removed. Its disk, tape, and other \
                 media files stay on disk.",
            );
            if running {
                ui.label(
                    egui::RichText::new(
                        "This machine is running — it will be shut down first, like \
                         flipping the power switch; unsaved work inside it is lost.",
                    )
                    .strong(),
                );
            }
            if let Some(err) = &self.delete_error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            ui.add_space(DETAIL_SECTION_GAP);
            ui.horizontal(|ui| {
                let confirm = if running { "Stop and Delete" } else { "Delete" };
                if ui.button(confirm).clicked() {
                    self.delete_machine(index);
                }
                if ui.button("Cancel").clicked() {
                    dismissed = true;
                }
            });
        });
        if dismissed || modal.should_close() {
            self.pending_delete = None;
            self.delete_error = None;
        }
    }

    /// Confirmed delete of `entries[index]`: stop its VM if one is running
    /// (same flush contract as the Stop button), remove its `<slug>.toml`,
    /// and drop the row. Media/artifact files are deliberately left on disk
    /// (the modal says so). Failure lands in [`Self::delete_error`] with the
    /// entry kept, so the still-open modal can retry or cancel.
    fn delete_machine(&mut self, index: usize) {
        let Some(dir) = self.machines_dir.clone() else {
            self.delete_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let path = dir.join(format!("{}.toml", self.entries[index].slug));
        // A file already gone (deleted externally since startup) is fine —
        // the goal state "no definition on disk" is reached either way.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                self.delete_error = Some(format!("{}: {e}", path.display()));
                return;
            }
        }
        self.stop_vm(index);
        self.entries.remove(index);
        match self.selected {
            Some(s) if s == index => {
                self.selected = None;
                self.edit = None;
            }
            Some(s) if s > index => self.selected = Some(s - 1),
            _ => {}
        }
        self.pending_delete = None;
        self.delete_error = None;
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
            let name = self.entries[i].def.name.clone();
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


/// Seed the detail pane's [`new_vm::MachineForm`] from a saved definition —
/// the inverse of [`ManagerApp::pack_def`], reconstructing the cartridge
/// picture the same way `crate::launch_machine` mounts it: with an MPI,
/// `[media].cart` re-seats in slot 0, the FD-502 in the last slot, the RTC
/// in its default slot; without one, the single port shows whichever of
/// pak/RTC/FD-502 the definition claims, in that priority (launch rejects a
/// conflicting combination outright — seeding at least shows one of them).
/// Disk media implies the FD-502 even when the flag is off (older files —
/// `launch_machine`'s rule).
fn seed_form(def: &machine_def::MachineDef) -> new_vm::MachineForm {
    let mut form = new_vm::MachineForm::new("detail", true);
    form.config = def.to_machine_config().expect("list entries are validated on load/save");
    let media = &def.media;
    let fd502 = def.peripherals.fd502 || media.disk0.is_some() || media.disk1.is_some();
    let cart = media.cart.as_deref().map(PathBuf::from);
    if def.peripherals.mpi {
        form.cartridge = new_vm::CartridgeChoice::MPI;
        if let Some(path) = cart {
            form.mpi_slots[0] = new_vm::SlotChoice::RomPak(path);
        }
        if fd502 {
            form.mpi_slots[crate::MPI_SLOT_COUNT - 1] = new_vm::SlotChoice::FD502;
        }
        if def.peripherals.rtc {
            form.mpi_slots[crate::DEFAULT_RTC_SLOT] = new_vm::SlotChoice::RTC;
        }
    } else if let Some(path) = cart {
        form.cartridge = new_vm::CartridgeChoice::RomPak(path);
    } else if def.peripherals.rtc {
        form.cartridge = new_vm::CartridgeChoice::RTC;
    } else if fd502 {
        form.cartridge = new_vm::CartridgeChoice::FD502;
    }
    let media_choice = |raw: &Option<String>| match raw {
        Some(s) => new_vm::MediaChoice::File(PathBuf::from(s)),
        None => new_vm::MediaChoice::None,
    };
    form.disks = [media_choice(&media.disk0), media_choice(&media.disk1)];
    form.tape = media_choice(&media.tape);
    form.vhds = [media_choice(&media.vhd0), media_choice(&media.vhd1)];
    form.aspect_correct = def.ui.aspect_correct;
    form.kb_mode = match def.ui.kb_mode {
        machine_def::KbModeDTO::Positional => crate::KbMode::Positional,
        machine_def::KbModeDTO::Symbolic => crate::KbMode::Symbolic,
    };
    form
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
