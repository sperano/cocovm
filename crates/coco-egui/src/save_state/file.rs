//! State files: Save to File… and Load from File… in the State selector
//! (`chrome::toolbar::quick_states`), for a `.ccstate` anywhere on disk
//! rather than one of the shared quick states. Neither changes the
//! selected state.

use std::path::Path;

use eframe::egui;

use crate::CocoApp;

use super::load_request::{LoadSource, file_name};

/// File-dialog filter name for save states.
const FILTER_NAME: &str = "CoCo save state";
/// Extension of a save state file.
const EXTENSION: &str = "ccstate";
/// File name the save dialog proposes.
const DEFAULT_FILE_NAME: &str = "state.ccstate";

impl CocoApp {
    /// Ask for a destination and save the machine state there.
    pub(crate) fn save_state_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(FILTER_NAME, &[EXTENSION])
            .set_file_name(DEFAULT_FILE_NAME)
            .save_file()
        {
            self.save_state_file(&path);
        }
    }

    /// Save the machine state to `path`: "Saved <file name>" on success,
    /// the error dialog on failure.
    pub(crate) fn save_state_file(&mut self, path: &Path) {
        match self.write_state_to(path) {
            Ok(()) => self.set_toast(format!("Saved {}", file_name(path))),
            Err(e) => self.cart_error = Some(e),
        }
    }

    /// Ask for a `.ccstate` and load it ([`Self::request_load`]).
    pub(crate) fn load_state_file_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(FILTER_NAME, &[EXTENSION])
            .pick_file()
        {
            self.request_load(LoadSource::File(path.clone()), &path, ctx);
        }
    }
}
