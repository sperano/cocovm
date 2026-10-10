//! "About" overlay window.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

/// Label of every menu item that opens the About window.
pub(crate) const MENU_LABEL: &str = "About CoCoVM";

const WINDOW_WIDTH: f32 = 360.0;
const CONTENT_MARGIN: i8 = 20;
const ICON_SIZE: f32 = 128.0;
const TITLE_SIZE: f32 = 30.0;
const DETAIL_SIZE: f32 = 12.0;
const TEXT_GAP: f32 = 6.0;
const SECTION_GAP: f32 = 16.0;
const ICON_BYTES: &[u8] = include_bytes!("../assets/cocovm-icon.png");
const ICON_CACHE_ID: &str = "about_cocovm_icon";
const GITHUB_URL: &str = "https://github.com/sperano/cocovm";

/// A request to open the About window, raised from outside the UI.
#[derive(Clone, Default)]
pub(crate) struct AboutRequest(Arc<AtomicBool>);

impl AboutRequest {
    /// Ask for the About window; the manager opens it on its next update.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn raise(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether a request was pending; clears it.
    pub(crate) fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// Draw the About window. `open` is toggled by the window's close box.
pub fn window(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(crate::window_title(ctx, MENU_LABEL))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_width(WINDOW_WIDTH)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .inner_margin(CONTENT_MARGIN)
                .show(ui, contents);
        });
}

fn contents(ui: &mut egui::Ui) {
    let icon = icon_texture(ui.ctx());
    ui.spacing_mut().item_spacing.y = TEXT_GAP;
    ui.vertical_centered(|ui| {
        ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::Vec2::splat(ICON_SIZE)));
        ui.label(egui::RichText::new("CoCoVM").size(TITLE_SIZE).strong());
        ui.label(concat!("Version ", env!("CARGO_PKG_VERSION")));
        ui.add_space(SECTION_GAP);
        ui.label("A Tandy Color Computer emulator");
        ui.label(egui::RichText::new("Built with Rust + egui").size(DETAIL_SIZE));
        ui.add_space(SECTION_GAP);
        ui.separator();
        ui.add_space(TEXT_GAP);
        ui.label(egui::RichText::new("© 2026 Éric Spérano").size(DETAIL_SIZE));
        ui.label(
            egui::RichText::new(concat!("License: ", env!("CARGO_PKG_LICENSE"))).size(DETAIL_SIZE),
        );
        ui.hyperlink_to("GitHub", GITHUB_URL);
    });
}

fn icon_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new(ICON_CACHE_ID);
    if let Some(icon) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return icon;
    }
    let image = image::load_from_memory(ICON_BYTES)
        .expect("embedded icon PNG is valid")
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let icon = ctx.load_texture(
        ICON_CACHE_ID,
        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, icon.clone()));
    icon
}
