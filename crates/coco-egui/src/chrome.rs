use crate::{CocoApp, egui};

mod menu_bar;
pub(crate) mod status_bar;
mod toolbar;
mod windows;

impl CocoApp {
    /// Draws every bit of chrome around the CoCo display. Split out of
    /// [`Self::window_ui`] so the `ViewportClass::Embedded` fallback can skip
    /// it — two apps sharing one `ctx` would interleave their menu bars.
    /// While `self.suspended` the bars stay visible but inert (menus and
    /// status popups disabled, transport tiles re-gated) and the optional
    /// windows are not drawn.
    pub(crate) fn draw_chrome(&mut self, ctx: &egui::Context) {
        self.menu_bar_ui(ctx);
        self.toolbar_ui(ctx);
        self.status_bar_ui(ctx);
        if !self.suspended {
            self.windows_ui(ctx);
        }
    }
}
