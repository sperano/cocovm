use crate::*;

/// Disabled-hover text for the Start tile — see [`CocoApp::toolbar_ui`]'s
/// doc for why Start is unconditionally disabled here.
const START_DISABLED_HOVER: &str = "This machine is already running";
/// Hover text for the Stop tile.
const STOP_HOVER: &str = "Shut down this machine — same as closing the window";
/// Hover text for the Reset tile.
const RESET_HOVER: &str = "Press the reset button";

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
    pub(crate) fn toolbar_ui(&mut self, ctx: &egui::Context) {
        // Explicit margin, not the default: keeps this in sync with `crate::TOOLBAR_H`'s
        // window-sizing math.
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(TOOLBAR_PANEL_MARGIN_X, TOOLBAR_PANEL_MARGIN_Y),
        );
        egui::TopBottomPanel::top("toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = BUTTON_GAP;

                    // Start is always disabled here: this window only exists while already Running.
                    let _ = toolbar_button(ui, PLAY_GLYPH, START_LABEL, false)
                        .on_disabled_hover_text(START_DISABLED_HOVER);

                    if toolbar_button(ui, SUSPEND_GLYPH, SUSPEND_LABEL, true)
                        .on_hover_text(SUSPEND_HOVER)
                        .clicked()
                    {
                        self.pending_suspend = true;
                    }

                    // Same path as the window's close box: routes through `stop_vm`, flushing
                    // dirty media.
                    if toolbar_button(ui, STOP_GLYPH, STOP_LABEL, true)
                        .on_hover_text(STOP_HOVER)
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    if toolbar_button(ui, RESET_GLYPH, RESET_LABEL, true)
                        .on_hover_text(RESET_HOVER)
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
                        if toolbar_button(ui, DEBUG_GLYPH, DEBUG_LABEL, true)
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
