//! Frontend save-state UX: Machine-menu Save/Load State, the quick states
//! (`quick.rs`) with their submenus and keyboard chords, and the status-bar
//! toast — all built on top of the engine in
//! [`coco_core::snapshot`], which this module is the only caller of.
//!
//! [`CocoApp::save_state_to`]/[`CocoApp::load_state_from`] are the two
//! entry points; everything else here is either UI chrome around them or the
//! fiddly frontend-side re-injection [`coco_core::snapshot::restore`] can't
//! do itself (host-only resources, path mirrors, pacing — see
//! [`CocoApp::apply_restored_machine`]).

use eframe::egui;

use crate::CocoApp;

#[cfg(test)]
mod fd502_test;
mod media_ref;
#[cfg(test)]
mod media_ref_test;
mod quick;
#[cfg(test)]
mod quick_test;
mod restore;
mod save;

pub(crate) use quick::{
    QUICK_SLOTS, StateFile, default_quick_state_dir, load_slot_shortcut, save_slot_shortcut,
    saved_time, slot_shortcuts_hint, state_name,
};
pub(crate) use save::DRIVEWIRE_HOST_BUSY;

/// How long a status-bar toast stays visible after [`CocoApp::set_toast`].
pub(crate) const TOAST_SECS: f64 = 4.0;

/// Disabled-hover text of a Load control whose state has no file yet.
pub(crate) fn empty_state_hover(slot: usize) -> String {
    format!("{} is empty. Save a state first.", state_name(slot))
}

/// `head`, then the restore `notes` (if any) after a colon — the shape of
/// every load toast.
fn with_notes(head: &str, notes: &[String]) -> String {
    if notes.is_empty() {
        head.to_string()
    } else {
        format!("{head}: {}", notes.join("; "))
    }
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
    /// the [`QUICK_SLOTS`] Quick Save/Quick Load submenus. Quick Load greys
    /// out states with no file.
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
                let button = self.quick_state_button(ui, slot, save_slot_shortcut);
                if ui.add(button).clicked() {
                    self.quick_save(slot);
                    ui.close();
                }
            }
        });
        ui.menu_button("Quick Load", |ui| {
            for slot in 0..QUICK_SLOTS {
                let occupied = !self.quick_state_file(slot).is_empty();
                let button = self.quick_state_button(ui, slot, load_slot_shortcut);
                if ui
                    .add_enabled(occupied, button)
                    .on_disabled_hover_text(empty_state_hover(slot))
                    .clicked()
                {
                    self.quick_load(slot, ui.ctx());
                    ui.close();
                }
            }
        });
    }

    /// One Quick Save/Quick Load submenu row: `slot`'s label, plus its chord
    /// when `shortcut` has one for it.
    fn quick_state_button(
        &self,
        ui: &egui::Ui,
        slot: usize,
        shortcut: fn(usize) -> Option<egui::KeyboardShortcut>,
    ) -> egui::Button<'static> {
        let button = egui::Button::new(self.quick_state_label(slot));
        match shortcut(slot) {
            Some(s) => button.shortcut_text(ui.ctx().format_shortcut(&s)),
            None => button,
        }
    }
}

#[cfg(test)]
#[path = "save_state_test.rs"]
pub(crate) mod tests;
