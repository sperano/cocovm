use crate::{CocoApp, egui};

mod menu_bar;
mod status_bar;
mod toolbar;
mod windows;

impl CocoApp {
    /// The menu bar, toolbar, status bar, and every optional window/dialog
    /// (keyboard help, About, the "New…" dialog, the printer-paper window,
    /// the disk-controller confirmation, the cartridge-error banner) — every
    /// bit of chrome around the CoCo display itself. Split out of
    /// [`Self::window_ui`] so the manager's `ViewportClass::Embedded`
    /// fallback can skip it entirely: drawing two apps' menu bars/status
    /// bars into one shared `ctx` would interleave them into a single
    /// confusing window, so that fallback shows only [`Self::draw_display`]
    /// (`docs/plan-machine-persistence.md` "one native window per running
    /// VM").
    pub(crate) fn draw_chrome(&mut self, ctx: &egui::Context) {
        self.menu_bar_ui(ctx);
        self.toolbar_ui(ctx);
        self.status_bar_ui(ctx);
        self.windows_ui(ctx);
    }
}
