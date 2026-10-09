use crate::{CocoApp, egui};

mod menus;
pub(crate) mod status_bar;
pub(crate) mod toolbar;
mod windows;

impl CocoApp {
    /// Draws every bit of chrome around the CoCo display. Split out of
    /// [`Self::window_ui`] so the `ViewportClass::Embedded` fallback can skip
    /// it — two apps sharing one `ctx` would interleave their toolbars.
    /// While `self.suspended` the bars stay visible but inert (status
    /// popups disabled, transport tiles re-gated) and the optional
    /// windows are not drawn.
    pub(crate) fn draw_chrome(&mut self, ctx: &egui::Context) {
        self.toolbar_ui(ctx);
        self.status_bar_ui(ctx);
        if !self.suspended {
            self.windows_ui(ctx);
        }
    }
}
