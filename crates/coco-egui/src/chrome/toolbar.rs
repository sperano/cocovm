use crate::*;

/// Hover text for the Start tile, only enabled while suspended.
const RESUME_HOVER: &str = "Resume this machine";
/// Disabled-hover text for the Start tile while the machine runs.
const START_DISABLED_HOVER: &str = "This machine is already running";
/// Disabled-hover text for the Suspend tile while suspended.
const SUSPEND_DISABLED_HOVER: &str = "This machine is already suspended";
/// Hover text for the Stop tile while running.
const STOP_HOVER: &str = "Shut down this machine — same as closing the window";
/// Hover text for the Stop tile while suspended: closing only drops the
/// window, the frozen state stays (the manager's Stop is what discards it).
const STOP_SUSPENDED_HOVER: &str = "Close this window — the machine stays suspended";
/// Hover text for the Reset tile.
const RESET_HOVER: &str = "Press the reset button";
/// Disabled-hover text for the Reset tile while suspended — resetting the
/// live object would desync it from the frozen `.ccstate`.
const RESET_DISABLED_HOVER: &str = "Resume the machine before resetting it";

/// Debug tile glyph — U+1F41E lady beetle, verified present in egui's
/// bundled NotoEmoji-Regular (monochrome, so it tints with the widget text
/// color like the manager toolbar's own emoji icons).
#[cfg(feature = "debug-ui")]
const DEBUG_GLYPH: &str = "🐞";
/// Caption under [`DEBUG_GLYPH`].
#[cfg(feature = "debug-ui")]
const DEBUG_LABEL: &str = "Debug";
/// Hover text for the Debug tile; [`CocoApp::toolbar_ui`] appends the
/// platform-formatted [`debugger::DEBUGGER_SHORTCUT`].
#[cfg(feature = "debug-ui")]
const DEBUG_HOVER: &str = "Open or close the debugger";

impl CocoApp {
    /// The VM window's toolbar: the transport tiles (Start/Suspend/Stop/Reset) plus a
    /// VM-only Debug tile. Keyboard/aspect controls live in menus, so aren't duplicated here.
    /// Start is only live while suspended (it resumes); Suspend, Reset and Debug only while
    /// running; Stop always.
    pub(crate) fn toolbar_ui(&mut self, ctx: &egui::Context) {
        let controllable = !self.suspended;
        // Explicit margin, not the default: keeps this in sync with `crate::TOOLBAR_H`'s
        // window-sizing math.
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(TOOLBAR_PANEL_MARGIN_X, TOOLBAR_PANEL_MARGIN_Y),
        );
        egui::TopBottomPanel::top("toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                let icons_only = self.toolbar_icons_only;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = BUTTON_GAP;

                    if toolbar_button(ui, PLAY_GLYPH, START_LABEL, self.suspended, icons_only)
                        .on_hover_text(RESUME_HOVER)
                        .on_disabled_hover_text(START_DISABLED_HOVER)
                        .clicked()
                    {
                        self.pending_resume = true;
                    }

                    if toolbar_button(ui, SUSPEND_GLYPH, SUSPEND_LABEL, controllable, icons_only)
                        .on_hover_text(SUSPEND_HOVER)
                        .on_disabled_hover_text(SUSPEND_DISABLED_HOVER)
                        .clicked()
                    {
                        self.pending_suspend = true;
                    }

                    // Same path as the window's close box: routes through `stop_vm`, flushing
                    // dirty media.
                    let stop_hover = if controllable {
                        STOP_HOVER
                    } else {
                        STOP_SUSPENDED_HOVER
                    };
                    if toolbar_button(ui, STOP_GLYPH, STOP_LABEL, true, icons_only)
                        .on_hover_text(stop_hover)
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    if toolbar_button(ui, RESET_GLYPH, RESET_LABEL, controllable, icons_only)
                        .on_hover_text(RESET_HOVER)
                        .on_disabled_hover_text(RESET_DISABLED_HOVER)
                        .clicked()
                    {
                        self.machine.reset();
                    }

                    #[cfg(feature = "debug-ui")]
                    {
                        toolbar_separator(ui);

                        // Same toggle as ⌘D; hover text formats the shortcut per-platform.
                        let debug_hover = format!(
                            "{DEBUG_HOVER} ({})",
                            ui.ctx().format_shortcut(&debugger::DEBUGGER_SHORTCUT)
                        );
                        if toolbar_button(ui, DEBUG_GLYPH, DEBUG_LABEL, controllable, icons_only)
                            .on_hover_text(debug_hover)
                            .clicked()
                        {
                            self.debugger.toggle();
                        }
                    }
                });
            });
    }
}
