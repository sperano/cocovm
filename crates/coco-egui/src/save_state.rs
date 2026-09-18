//! Frontend save-state UX: Machine-menu Save/Load State + Quick Save/Load
//! slots, their keyboard
//! chords, and the status-bar toast — all built on top of the engine in
//! [`coco_core::snapshot`], which this module is the only caller of.
//!
//! [`CocoApp::save_state_to`]/[`CocoApp::load_state_from`] are the two
//! entry points; everything else here is either UI chrome around them or the
//! fiddly frontend-side re-injection [`coco_core::snapshot::restore`] can't
//! do itself (host-only resources, path mirrors, pacing — see
//! [`CocoApp::apply_restored_machine`]).

use std::path::PathBuf;

use eframe::egui;

use crate::{CocoApp, paths};

mod media_ref;
#[cfg(test)]
mod media_ref_test;
mod restore;
mod save;

pub(crate) use save::DRIVEWIRE_HOST_BUSY;

/// Number of quick-save/quick-load slots the Machine menu exposes.
pub(crate) const QUICK_SLOTS: usize = 3;

/// Subdirectory of [`paths::data_dir`] holding quick-save slot files
/// (`<dir>/slot-<n>.ccstate`, 1-based).
const SAVE_STATES_SUBDIR: &str = "save-states";

/// How long a status-bar toast stays visible after [`CocoApp::set_toast`].
pub(crate) const TOAST_SECS: f64 = 4.0;

/// Physical keys `slot` (0-based) binds to: `Num1`/`Num2`/`Num3` for the
/// three [`QUICK_SLOTS`].
const QUICK_SLOT_KEYS: [egui::Key; QUICK_SLOTS] =
    [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3];

/// COMMAND+SHIFT+`<n>` quick-saves state slot `slot` — the SHIFTed sibling
/// of [`load_slot_shortcut`]'s COMMAND+`<n>`.
pub(crate) fn save_slot_shortcut(slot: usize) -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        QUICK_SLOT_KEYS[slot],
    )
}

/// COMMAND+`<n>` quick-loads state slot `slot`.
pub(crate) fn load_slot_shortcut(slot: usize) -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, QUICK_SLOT_KEYS[slot])
}

/// One line for the keyboard-help window naming every quick-slot chord,
/// formatted per-platform using [`egui::Context::format_shortcut`].
pub(crate) fn slot_shortcuts_hint(ctx: &egui::Context) -> String {
    let loads: Vec<String> = (0..QUICK_SLOTS)
        .map(|s| ctx.format_shortcut(&load_slot_shortcut(s)))
        .collect();
    let saves: Vec<String> = (0..QUICK_SLOTS)
        .map(|s| ctx.format_shortcut(&save_slot_shortcut(s)))
        .collect();
    format!(
        "{}: quick-load state slot 1/2/3   ·   {}: quick-save",
        loads.join(" / "),
        saves.join(" / ")
    )
}

/// `<data_dir>/save-states/slot-<n>.ccstate` (1-based) for `slot` (0-based).
/// `None` when [`paths::data_dir`] can't determine a home directory.
fn quick_slot_path(slot: usize) -> Option<PathBuf> {
    let dir = paths::data_dir()?.join(SAVE_STATES_SUBDIR);
    Some(dir.join(format!("slot-{}.ccstate", slot + 1)))
}

/// Menu-item label for `slot`: its last-modified time if the slot file
/// exists, else "(empty)".
fn quick_slot_label(slot: usize) -> String {
    let n = slot + 1;
    let mtime = quick_slot_path(slot)
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    match mtime {
        Some(t) => format!("Slot {n} ({})", format_mtime(t)),
        None => format!("Slot {n} (empty)"),
    }
}

fn format_mtime(t: std::time::SystemTime) -> String {
    let local: chrono::DateTime<chrono::Local> = t.into();
    local.format("%Y-%m-%d %H:%M").to_string()
}

impl CocoApp {
    /// Show `msg` in the status bar for [`TOAST_SECS`] seconds.
    fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    /// The current toast text, if one is still within [`TOAST_SECS`] of
    /// [`Self::set_toast`] — clears itself once expired.
    pub(crate) fn toast_message(&mut self) -> Option<String> {
        let (msg, at) = self.toast.as_ref()?;
        if at.elapsed().as_secs_f64() > TOAST_SECS {
            self.toast = None;
            return None;
        }
        Some(msg.clone())
    }

    /// The Machine menu's Save/Load State section: file-dialog Save/Load plus
    /// the [`QUICK_SLOTS`] Quick Save/Quick Load submenus.
    pub(crate) fn draw_save_state_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Save State…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CoCo save state", &["ccstate"])
                .set_file_name("state.ccstate")
                .save_file()
                && let Err(e) = self.save_state_to(&path)
            {
                self.cart_error = Some(e);
            }
        }
        if ui.button("Load State…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("CoCo save state", &["ccstate"])
                .pick_file()
            {
                match self.load_state_from(&path) {
                    Ok(()) => self.refresh_window_title(ui.ctx()),
                    Err(e) => self.cart_error = Some(e),
                }
            }
        }
        ui.menu_button("Quick Save", |ui| {
            for slot in 0..QUICK_SLOTS {
                let button = egui::Button::new(quick_slot_label(slot))
                    .shortcut_text(ui.ctx().format_shortcut(&save_slot_shortcut(slot)));
                if ui.add(button).clicked() {
                    self.quick_save(slot);
                    ui.close();
                }
            }
        });
        ui.menu_button("Quick Load", |ui| {
            for slot in 0..QUICK_SLOTS {
                let occupied = quick_slot_path(slot).is_some_and(|p| p.is_file());
                let button = egui::Button::new(quick_slot_label(slot))
                    .shortcut_text(ui.ctx().format_shortcut(&load_slot_shortcut(slot)));
                if ui.add_enabled(occupied, button).clicked() {
                    self.quick_load(slot, ui.ctx());
                    ui.close();
                }
            }
        });
    }

    /// Quick Save `slot`: like "Save State…" but to a fixed per-slot path
    /// under [`paths::data_dir`], creating [`SAVE_STATES_SUBDIR`] on demand.
    pub(crate) fn quick_save(&mut self, slot: usize) {
        let Some(path) = quick_slot_path(slot) else {
            self.cart_error = Some("no data directory found for quick-save slots".to_string());
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            self.cart_error = Some(format!("could not create {}: {e}", dir.display()));
            return;
        }
        if let Err(e) = self.save_state_to(&path) {
            self.cart_error = Some(e);
        }
    }

    /// Quick Load `slot` — the load-side sibling of [`Self::quick_save`].
    pub(crate) fn quick_load(&mut self, slot: usize, ctx: &egui::Context) {
        let Some(path) = quick_slot_path(slot) else {
            self.cart_error = Some("no data directory found for quick-save slots".to_string());
            return;
        };
        match self.load_state_from(&path) {
            Ok(()) => self.refresh_window_title(ctx),
            Err(e) => self.cart_error = Some(e),
        }
    }
}

#[cfg(test)]
#[path = "save_state_test.rs"]
pub(crate) mod tests;
