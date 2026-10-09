//! Confirm application exit before the native root viewport closes.

use eframe::egui;

use super::{MachineEntry, ManagerApp};

const DIALOG_WIDTH: f32 = 520.0;
const MACHINE_LIST_HEIGHT: f32 = 320.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ExitAction {
    #[default]
    Suspend,
    ShutDown,
}

struct ExitChoice {
    session: u64,
    action: ExitAction,
    error: Option<String>,
    /// Stop can drop a VM before reporting a checkpoint-discard failure.
    retry_shutdown: bool,
}

#[derive(Default)]
pub(super) struct ExitState {
    choices: Option<Vec<ExitChoice>>,
    approved: bool,
    /// A discarded layout pass must not count as showing new choices.
    choices_changed_frame: Option<u64>,
}

impl ExitState {
    pub(super) fn is_pending(&self) -> bool {
        self.choices.is_some()
    }

    /// Returns whether a running machine was added and needs to be shown
    /// before the user can confirm. Session identities survive rename/sort.
    fn reconcile(&mut self, entries: &[MachineEntry]) -> bool {
        let Some(choices) = self.choices.as_mut() else {
            return false;
        };
        choices.retain(|choice| {
            entries.iter().any(|entry| {
                entry.window_session == choice.session
                    && (entry.is_running() || choice.retry_shutdown)
            })
        });
        let mut added = false;
        for entry in entries.iter().filter(|entry| entry.is_running()) {
            if let Some(choice) = choices
                .iter_mut()
                .find(|c| c.session == entry.window_session)
            {
                if choice.retry_shutdown {
                    // A stopped VM was restarted while cleanup was pending.
                    // Show fresh choices before acting on this live session.
                    choice.action = ExitAction::default();
                    choice.retry_shutdown = false;
                    choice.error = None;
                    added = true;
                }
            } else {
                choices.push(ExitChoice {
                    session: entry.window_session,
                    action: ExitAction::default(),
                    error: None,
                    retry_shutdown: false,
                });
                added = true;
            }
        }
        added
    }
}

impl ManagerApp {
    /// `on_exit` is too late to cancel: intercept root Close during update.
    /// Once approved, no UI or control request may start another machine.
    pub(super) fn handle_exit_request(&mut self, ctx: &egui::Context) -> bool {
        if self.exit.approved {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return true;
        }
        if !ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }
        if !self.exit.is_pending() && !self.entries.iter().any(MachineEntry::is_running) {
            self.approve_exit(ctx);
            return true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.exit.choices.get_or_insert_with(Vec::new);
        false
    }

    pub(super) fn draw_exit_confirmation(&mut self, ctx: &egui::Context) {
        if !self.exit.is_pending() {
            return;
        }
        let added = self.exit.reconcile(&self.entries);
        let frame = ctx.cumulative_frame_nr();
        if added {
            self.exit.choices_changed_frame = Some(frame);
        }
        let can_confirm = self.exit.choices_changed_frame != Some(frame);
        let mut cancel = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_app_exit")).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.heading("Quit CoCoVM?");
            ui.label("Choose what happens to each running machine.");
            ui.label("Suspend saves the machine's state so you can resume it later.");
            ui.label("Shut down turns off the machine. Unsaved work inside it is lost.");
            self.draw_exit_choices(ui);
            ui.horizontal(|ui| {
                cancel = ui.button("Cancel").clicked();
                confirm = ui
                    .add_enabled(can_confirm, egui::Button::new("Quit"))
                    .clicked();
            });
        });
        if cancel || modal.should_close() {
            self.exit = ExitState::default();
        } else if confirm {
            self.commit_exit(ctx);
        }
        if added {
            ctx.request_repaint();
        }
    }

    fn draw_exit_choices(&mut self, ui: &mut egui::Ui) {
        let choices = self.exit.choices.as_mut().expect("exit dialog is open");
        egui::ScrollArea::vertical()
            .max_height(MACHINE_LIST_HEIGHT)
            .show(ui, |ui| {
                for choice in choices {
                    let Some(entry) = self
                        .entries
                        .iter()
                        .find(|entry| entry.window_session == choice.session)
                    else {
                        continue;
                    };
                    ui.push_id(choice.session, |ui| {
                        ui.separator();
                        ui.strong(&entry.def.name);
                        if choice.retry_shutdown {
                            ui.label("Shut down: cleanup needs to be retried.");
                        } else {
                            ui.horizontal(|ui| {
                                ui.radio_value(
                                    &mut choice.action,
                                    ExitAction::Suspend,
                                    "Suspend and close",
                                );
                                ui.radio_value(
                                    &mut choice.action,
                                    ExitAction::ShutDown,
                                    "Shut down",
                                );
                            });
                        }
                        if let Some(error) = &choice.error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                }
            });
    }

    fn commit_exit(&mut self, ctx: &egui::Context) {
        if self.exit.reconcile(&self.entries) {
            ctx.request_repaint();
            return;
        }
        let choices = self.exit.choices.take().expect("exit dialog is open");
        let mut failed = Vec::new();
        for mut choice in choices {
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| entry.window_session == choice.session)
            else {
                continue;
            };
            match self.apply_exit_action(index, choice.action) {
                Ok(()) => {}
                Err(error) => {
                    choice.error = Some(error);
                    choice.retry_shutdown =
                        choice.action == ExitAction::ShutDown && !self.entries[index].is_running();
                    failed.push(choice);
                }
            }
        }
        self.exit.choices = Some(failed);
        self.exit.reconcile(&self.entries);
        if self.exit.choices.as_ref().is_some_and(Vec::is_empty) {
            self.approve_exit(ctx);
        }
    }

    fn apply_exit_action(&mut self, index: usize, action: ExitAction) -> Result<(), String> {
        match action {
            ExitAction::Suspend => {
                self.suspend_vm(index);
                if let Some(error) = &self.entries[index].launch_error {
                    return Err(error.clone());
                }
                self.close_vm_window(index);
            }
            ExitAction::ShutDown => {
                // Stop deliberately drops the VM even on flush failure. Preflight
                // here so a failed quit leaves unsaved media available to retry.
                if let Some(vm) = self.entries[index].vm.as_mut() {
                    vm.flush_media()?;
                }
                self.stop_vm(index);
                if let Some(error) = &self.entries[index].launch_error {
                    return Err(error.clone());
                }
            }
        }
        Ok(())
    }

    fn approve_exit(&mut self, ctx: &egui::Context) {
        self.exit.approved = true;
        self.exit.choices = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }
}

#[cfg(test)]
#[path = "exit_test.rs"]
mod tests;
