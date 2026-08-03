use crate::*;

/// Disabled-hover text for the Start tile — see [`CocoApp::toolbar_ui`]'s
/// doc for why Start is unconditionally disabled here.
const START_DISABLED_HOVER: &str = "This machine is already running";
/// Hover text for the Stop tile.
const STOP_HOVER: &str = "Shut down this machine — same as closing the window";
/// Hover text for the Reset tile.
const RESET_HOVER: &str = "Press the reset button";

impl CocoApp {
    /// The VM window's toolbar: the same four transport tiles
    /// (Start/Suspend/Stop/Reset) the manager window's own toolbar draws,
    /// built from the same shared [`toolbar_button`] widget
    /// (`widgets.rs`) — so a launched machine's own window presents the
    /// identical transport row the manager does. "⌨ Keys (F10)" and
    /// "4:3 (F9)", which used to live here, are not relocated: they already
    /// exist as the Keyboard menu's "Key layout (F10)" and the View menu's
    /// "4:3 aspect (F9)" (plus their F-key hotkeys), so this row no longer
    /// duplicates them.
    pub(crate) fn toolbar_ui(&mut self, ctx: &egui::Context) {
        // Explicit rather than relying on `TopBottomPanel`'s own default
        // frame: this pins the panel's inner margin to our named constants
        // ([`TOOLBAR_PANEL_MARGIN_X`]/[`TOOLBAR_PANEL_MARGIN_Y`]) — the same
        // values [`crate::TOOLBAR_H`] uses to size the VM window — so an
        // egui upgrade that changes `Frame::side_top_panel`'s own default
        // can't silently desync the window-sizing math from what actually
        // renders. Built from `side_top_panel` (rather than from scratch) so
        // everything but the margin — fill color included — still matches
        // egui's other panels.
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(
            egui::Margin::symmetric(TOOLBAR_PANEL_MARGIN_X, TOOLBAR_PANEL_MARGIN_Y),
        );
        egui::TopBottomPanel::top("toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = BUTTON_GAP;

                    // A chrome-bearing VM window only ever exists while the
                    // machine is Running: a suspended window is display-only
                    // (`manager::vm_windows` draws no chrome for it), and there
                    // is no window at all for a Powered Off machine. So Start
                    // has nothing to do here — it's shown anyway only so this
                    // toolbar and the manager's present the same transport row.
                    let _ = toolbar_button(ui, PLAY_GLYPH, START_LABEL, false)
                        .on_disabled_hover_text(START_DISABLED_HOVER);

                    if toolbar_button(ui, SUSPEND_GLYPH, SUSPEND_LABEL, true)
                        .on_hover_text(SUSPEND_HOVER)
                        .clicked()
                    {
                        self.pending_suspend = true;
                    }

                    // Deliberately the same path as the window's close box,
                    // already documented as the power switch elsewhere: this
                    // window's close routes to `stop_vm`
                    // (`manager::vm_windows::close_vm_window`), which flushes
                    // dirty media before dropping the VM. `ctx` here is this
                    // (child) viewport's own context, so the command targets
                    // this VM window, not the manager.
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
                });
            });
    }
}
