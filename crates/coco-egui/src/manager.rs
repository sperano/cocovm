//! The CocoVM manager window: the VirtualBox/Parallels-style main window a
//! bare `coco` (no CLI arguments) opens instead of booting a machine
//! directly. Toolbar across the top, machine list down the left (one row per
//! `config_dir()/machines/<slug>.toml`, `machine_def.rs`), and a detail/edit
//! pane on the right for the selected machine — or, with no machine
//! selected, a random photo asset filling the pane.
//!
//! Launching a machine (`plan-machine-persistence.md` step 5+) isn't wired
//! up yet: "New…" creates a definition file but never boots it, and there is
//! no "Start" action. The direct-boot emulator (`CocoApp`) is untouched and
//! still serves every CLI invocation with arguments.

use std::path::PathBuf;

use coco_core::MachineConfig;
use eframe::egui;

use crate::photo_view::{self, Photo};
use crate::{machine_def, new_vm};

/// Manager window size at first open.
const WINDOW_SIZE: [f32; 2] = [1080.0, 720.0];

/// Machine-list panel: width at first open and the draggable divider's range.
const LIST_DEFAULT_WIDTH: f32 = 260.0;
const LIST_MIN_WIDTH: f32 = 160.0;
const LIST_MAX_WIDTH: f32 = 520.0;

/// Fixed size of a list row's placeholder art. Step 6 of the plan
/// (`docs/plan-machine-persistence.md`) replaces this with a live miniature
/// of the VM's own framebuffer texture (running/paused) or a saved
/// `thumbnail.png` (stopped) — until then every row gets the same dark rect.
const ROW_THUMBNAIL_SIZE: egui::Vec2 = egui::vec2(48.0, 36.0);
/// Corner rounding of the row thumbnail placeholder itself — distinct from
/// [`ROW_CORNER_RADIUS`], the row's own selection/hover frame.
const THUMBNAIL_CORNER_RADIUS: f32 = 2.0;
/// Fill of the row thumbnail placeholder, until step 6 of the plan
/// (`docs/plan-machine-persistence.md`) replaces it with a live/saved image.
const THUMBNAIL_PLACEHOLDER_FILL: egui::Color32 = egui::Color32::from_gray(30);
/// Inner padding of one list row's frame.
const ROW_MARGIN: f32 = 8.0;
/// Corner rounding of a list row's selection/hover frame.
const ROW_CORNER_RADIUS: f32 = 4.0;

/// List-row status label. Runtime state (running/paused/stopped) is never
/// persisted (`plan-machine-persistence.md` "Decisions") and no machine can
/// be launched from the manager yet, so every row reads the same thing for
/// now.
const STATUS_STOPPED: &str = "Stopped";

/// Vertical gap between sections of the detail pane.
const DETAIL_SECTION_GAP: f32 = 12.0;

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
    /// `photo`, `machines_dir`, and `entries` are all injected (rather than
    /// loaded here) so tests can construct the manager without touching the
    /// user's real config/asset directories.
    pub fn new(photo: Option<Photo>, machines_dir: Option<PathBuf>, entries: Vec<MachineEntry>) -> Self {
        Self {
            photo,
            photo_texture: None,
            machines_dir,
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
                self.entries.insert(index, MachineEntry { slug, def: Ok(def) });
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
                    let (rect, _) = ui.allocate_exact_size(ROW_THUMBNAIL_SIZE, egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, THUMBNAIL_CORNER_RADIUS, THUMBNAIL_PLACEHOLDER_FILL);

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
                                ui.weak(STATUS_STOPPED);
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
        edit.def.hardware = machine_def::HardwareDto::from_config(&config, edit.def.hardware.rom.clone());

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
            ui.radio_value(&mut edit.def.ui.kb_mode, machine_def::KbModeDto::Positional, "Positional");
            ui.radio_value(&mut edit.def.ui.kb_mode, machine_def::KbModeDto::Symbolic, "Symbolic");
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
        def.hardware = machine_def::HardwareDto::from_config(&config, def.hardware.rom.clone());
    }
    def
}

/// The `[media]` fields that are set, as `(row label, value)` pairs, in
/// schema-declaration order.
fn media_rows(media: &machine_def::MediaDto) -> Vec<(&'static str, &str)> {
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
                .map(|(slug, def)| MachineEntry { slug, def })
                .collect()
        })
        .unwrap_or_default();
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(ManagerApp::new(photo_view::random(), machines_dir, entries)))
        }),
    )
}
