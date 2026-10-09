//! The Settings dialog's Hotkeys section (`manager/settings.rs`): one row
//! per [`HotkeyAction::active`] action showing its binding as a button.
//! Clicking the button captures the next key press as the new binding;
//! Esc cancels the capture without closing the dialog.

use eframe::egui;

use crate::hotkeys::{DEFAULT_HOTKEYS, Hotkey, HotkeyAction, Hotkeys};

/// The binding button's text while it waits for a key.
pub(super) const CAPTURE_PROMPT: &str = "Press a key…";
/// Each row's restore-default button.
pub(super) const RESET_LABEL: &str = "Reset";
/// Minimum width of a binding button, so the column doesn't jump between
/// `F10` and [`CAPTURE_PROMPT`].
const BINDING_BUTTON_WIDTH: f32 = 110.0;

/// The draft bindings plus the capture in progress.
pub(super) struct HotkeyEditor {
    pub(super) hotkeys: Hotkeys,
    /// The action whose button is waiting for a key press.
    capturing: Option<HotkeyAction>,
    /// Why the last captured key was refused, until the next capture.
    error: Option<String>,
}

impl HotkeyEditor {
    pub(super) fn new(hotkeys: Hotkeys) -> Self {
        Self {
            hotkeys,
            capturing: None,
            error: None,
        }
    }

    /// One row per action and any capture error.
    pub(super) fn draw(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("settings_hotkeys")
            .num_columns(3)
            .show(ui, |ui| {
                for action in HotkeyAction::active() {
                    self.draw_row(ui, action);
                    ui.end_row();
                }
            });
        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
    }

    fn draw_row(&mut self, ui: &mut egui::Ui, action: HotkeyAction) {
        ui.label(action.label());
        let capturing = self.capturing == Some(action);
        let text = if capturing {
            CAPTURE_PROMPT.to_string()
        } else {
            ui.ctx()
                .format_shortcut(&self.hotkeys.get(action).shortcut())
        };
        let button = egui::Button::new(text)
            .selected(capturing)
            .min_size(egui::vec2(BINDING_BUTTON_WIDTH, 0.0));
        if ui
            .add(button)
            .on_hover_text(format!(
                "Click, then press the new {} hotkey",
                action.label()
            ))
            .clicked()
        {
            self.capturing = (!capturing).then_some(action);
            self.error = None;
        }
        let default = DEFAULT_HOTKEYS.get(action);
        if ui
            .add_enabled(
                self.hotkeys.get(action) != default,
                egui::Button::new(RESET_LABEL),
            )
            .on_hover_text(format!(
                "Restore {}",
                ui.ctx().format_shortcut(&default.shortcut())
            ))
            .clicked()
        {
            self.hotkeys.set(action, default);
            self.capturing = None;
            // Another action may have taken the default meanwhile; Save refuses that.
            self.error = self.hotkeys.holder(default, action).map(|other| {
                format!(
                    "{default} is also the {} hotkey: change one before saving",
                    other.label()
                )
            });
        }
    }

    pub(super) fn cancel_capture(&mut self) {
        self.capturing = None;
    }

    /// Called before any dialog widget, so a capture cannot activate a tab or button.
    /// While capturing, takes the frame's first key press out of the
    /// input: Esc cancels; anything else becomes the binding, unless
    /// [`Hotkey::from_press`] refuses it or another action already has it.
    pub(super) fn take_captured_key(&mut self, ui: &egui::Ui) {
        let Some(action) = self.capturing else {
            return;
        };
        let Some(press) = ui.input_mut(take_first_press) else {
            return;
        };
        self.capturing = None;
        if press.key == egui::Key::Escape && press.modifiers.is_none() {
            self.error = None;
            return;
        }
        match Hotkey::from_press(press.key, press.physical_key, press.modifiers) {
            Ok(hotkey) => match self.hotkeys.holder(hotkey, action) {
                Some(other) => {
                    self.error = Some(format!("{hotkey} is already the {} hotkey", other.label()));
                }
                None => {
                    self.hotkeys.set(action, hotkey);
                    self.error = None;
                }
            },
            Err(e) => self.error = Some(e),
        }
    }
}

/// A captured key press: what [`Hotkey::from_press`] needs from the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Press {
    key: egui::Key,
    physical_key: Option<egui::Key>,
    modifiers: egui::Modifiers,
}

/// Removes and returns the first fresh key press in `input`.
fn take_first_press(input: &mut egui::InputState) -> Option<Press> {
    let index = input.events.iter().position(|event| {
        matches!(
            event,
            egui::Event::Key {
                pressed: true,
                repeat: false,
                ..
            }
        )
    })?;
    match input.events.remove(index) {
        egui::Event::Key {
            key,
            physical_key,
            modifiers,
            ..
        } => Some(Press {
            key,
            physical_key,
            modifiers,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "hotkeys_test.rs"]
mod tests;
