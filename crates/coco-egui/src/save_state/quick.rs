//! Quick states: the [`QUICK_SLOTS`] numbered save-state files every VM
//! window shares, their labels and keyboard chords, and the quick
//! save/load actions behind the toolbar's state group
//! (`chrome::toolbar::quick_states`) and the numbered chords
//! (`app/input.rs`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui;

use crate::{CocoApp, paths};

use super::load_request::LoadSource;

/// Number of quick states (State 1 to State 10). Ten gives room for several
/// experiments without a long selector; named state files cover the rest.
pub(crate) const QUICK_SLOTS: usize = 10;

/// Subdirectory of [`paths::data_dir`] holding the quick-state files
/// (`<dir>/slot-<n>.ccstate`, 1-based). The file names predate the "State"
/// wording and stay as they are so existing saves keep loading.
const SAVE_STATES_SUBDIR: &str = "save-states";

/// Physical keys of the numbered chords. Only States 1 to 3 have one: users
/// may already bind COMMAND+4 and up to their own hotkeys.
const QUICK_SLOT_KEYS: [egui::Key; 3] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3];

/// Error when [`paths::data_dir`] can't locate a home directory.
const NO_DATA_DIR: &str = "no data directory found for quick states";

/// COMMAND+SHIFT+`<n>` quick-saves `slot` (0-based) — the SHIFTed sibling of
/// [`load_slot_shortcut`]'s COMMAND+`<n>`. `None` past the chorded states.
pub(crate) fn save_slot_shortcut(slot: usize) -> Option<egui::KeyboardShortcut> {
    let modifiers = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
    QUICK_SLOT_KEYS
        .get(slot)
        .map(|&key| egui::KeyboardShortcut::new(modifiers, key))
}

/// COMMAND+`<n>` quick-loads `slot` (0-based). `None` past the chorded states.
pub(crate) fn load_slot_shortcut(slot: usize) -> Option<egui::KeyboardShortcut> {
    QUICK_SLOT_KEYS
        .get(slot)
        .map(|&key| egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key))
}

/// One line for the keyboard-help window naming every quick-state chord,
/// formatted per-platform using [`egui::Context::format_shortcut`].
pub(crate) fn slot_shortcuts_hint(ctx: &egui::Context) -> String {
    let format = |shortcut: fn(usize) -> Option<egui::KeyboardShortcut>| {
        (0..QUICK_SLOTS)
            .filter_map(shortcut)
            .map(|s| ctx.format_shortcut(&s))
            .collect::<Vec<_>>()
            .join(" / ")
    };
    format!(
        "{}: quick-load State 1/2/3   ·   {}: quick-save",
        format(load_slot_shortcut),
        format(save_slot_shortcut)
    )
}

/// `<data_dir>/save-states`, where [`CocoApp::quick_state_dir`] starts.
/// `None` when [`paths::data_dir`] can't determine a home directory.
pub(crate) fn default_quick_state_dir() -> Option<PathBuf> {
    paths::data_dir().map(|dir| dir.join(SAVE_STATES_SUBDIR))
}

/// `slot`'s (0-based) file in `dir`.
fn state_file(dir: &Path, slot: usize) -> PathBuf {
    dir.join(format!("slot-{}.ccstate", slot + 1))
}

/// User-visible name of `slot` (0-based): "State 1" to "State 10".
pub(crate) fn state_name(slot: usize) -> String {
    format!("State {}", slot + 1)
}

/// Toast for a load shortcut aimed at an empty state.
pub(crate) fn empty_state_toast(slot: usize) -> String {
    format!("{} is empty", state_name(slot))
}

/// What a quick state's file looks like on disk right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateFile {
    /// No file: nothing to load.
    Empty,
    /// A file exists, with its modification time when readable. Whether it
    /// actually loads is up to the restore path, which reports failures.
    Saved(Option<SystemTime>),
}

impl StateFile {
    /// Probe `path`. Only a missing file counts as [`StateFile::Empty`]; any
    /// other metadata failure means something is there for Load to report on.
    pub(crate) fn probe(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => Self::Saved(meta.modified().ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::Empty,
            Err(_) => Self::Saved(None),
        }
    }

    pub(crate) fn is_empty(self) -> bool {
        self == Self::Empty
    }
}

/// When a saved state was written, relative to `now`: "22:40" today,
/// "2026-10-05 22:40" on any other day.
pub(crate) fn saved_time(t: SystemTime, now: chrono::DateTime<chrono::Local>) -> String {
    let local: chrono::DateTime<chrono::Local> = t.into();
    if local.date_naive() == now.date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// Selector row for `slot` (0-based): "State 1 — saved 22:40", "State 2 — Empty",
/// or "State 3 — timestamp unavailable" when the file's time can't be read.
pub(crate) fn state_row_label(
    slot: usize,
    file: StateFile,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let name = state_name(slot);
    match file {
        StateFile::Empty => format!("{name} — Empty"),
        StateFile::Saved(Some(t)) => format!("{name} — saved {}", saved_time(t, now)),
        StateFile::Saved(None) => format!("{name} — timestamp unavailable"),
    }
}

impl CocoApp {
    /// `slot`'s file under [`Self::quick_state_dir`].
    fn quick_state_path(&self, slot: usize) -> Option<PathBuf> {
        self.quick_state_dir
            .as_deref()
            .map(|dir| state_file(dir, slot))
    }

    /// Probe `slot`'s file afresh. Cheap enough (one `stat`) to call every
    /// frame, which is what keeps a save from another VM window, or a
    /// deleted file, reflected here without any cache to go stale.
    pub(crate) fn quick_state_file(&self, slot: usize) -> StateFile {
        self.quick_state_path(slot)
            .map_or(StateFile::Empty, |path| StateFile::probe(&path))
    }

    /// [`state_row_label`] for `slot` as it stands now.
    pub(crate) fn quick_state_label(&self, slot: usize) -> String {
        state_row_label(slot, self.quick_state_file(slot), chrono::Local::now())
    }

    /// Quick Save `slot`: like Save to File… but to the state's fixed file,
    /// replacing any earlier save. On success, toasts "Saved State N" and
    /// makes `slot` this window's selected state; on failure, reports the
    /// error dialog and leaves the selection alone. Returns whether it saved.
    pub(crate) fn quick_save(&mut self, slot: usize) -> bool {
        match self.write_quick_state(slot) {
            Ok(()) => {
                self.set_toast(format!("Saved {}", state_name(slot)));
                self.selected_quick_state = slot;
                true
            }
            Err(e) => {
                self.cart_error = Some(e);
                false
            }
        }
    }

    /// Create the quick-state directory on demand and write `slot`'s file.
    fn write_quick_state(&mut self, slot: usize) -> Result<(), String> {
        let path = self
            .quick_state_path(slot)
            .ok_or_else(|| NO_DATA_DIR.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        self.write_state_to(&path)
    }

    /// Quick Load `slot` — the load-side sibling of [`Self::quick_save`],
    /// through [`Self::request_load`]: "Loaded State N" plus any restore
    /// notes on success, the machine-type prompt first when the state was
    /// saved on another type, the error dialog on failure.
    pub(crate) fn quick_load(&mut self, slot: usize, ctx: &egui::Context) {
        match self.quick_state_path(slot) {
            Some(path) => self.request_load(LoadSource::Quick(slot), &path, ctx),
            None => self.cart_error = Some(NO_DATA_DIR.to_string()),
        }
    }

    /// The numbered load chord: [`Self::quick_load`], except that an empty
    /// state skips the load and only toasts that it is empty.
    pub(crate) fn quick_load_shortcut(&mut self, slot: usize, ctx: &egui::Context) {
        if self.quick_state_file(slot).is_empty() {
            self.set_toast(empty_state_toast(slot));
        } else {
            self.quick_load(slot, ctx);
        }
    }
}
