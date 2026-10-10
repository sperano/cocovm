//! "About" overlay window.

use eframe::egui;

use crate::update::{Status, UpdateCheck};

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

/// The About window's update line while a check runs.
pub(crate) const CHECKING_TEXT: &str = "Checking for updates…";
/// The update line when the latest release is not newer than this build.
pub(crate) const UP_TO_DATE_TEXT: &str = "CoCoVM is up to date.";
/// The update line after a requested check failed; the error is its hover text.
pub(crate) const CHECK_FAILED_TEXT: &str = "Could not check for updates.";

/// Draw the About window. `open` is toggled by the window's close box;
/// `inventory` is the installed-asset and machine count line
/// ([`crate::startup::inventory`]); `update` supplies the line under the
/// version.
pub fn window(ctx: &egui::Context, open: &mut bool, inventory: &str, update: &UpdateCheck) {
    egui::Window::new(crate::window_title(ctx, MENU_LABEL))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_width(WINDOW_WIDTH)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .inner_margin(CONTENT_MARGIN)
                .show(ui, |ui| contents(ui, inventory, update));
        });
}

fn contents(ui: &mut egui::Ui, inventory: &str, update: &UpdateCheck) {
    let icon = icon_texture(ui.ctx());
    ui.spacing_mut().item_spacing.y = TEXT_GAP;
    ui.vertical_centered(|ui| {
        ui.add(egui::Image::new(&icon).fit_to_exact_size(egui::Vec2::splat(ICON_SIZE)));
        ui.label(egui::RichText::new("CoCoVM").size(TITLE_SIZE).strong());
        ui.label(concat!("Version ", env!("CARGO_PKG_VERSION")));
        update_line(ui, update);
        ui.add_space(SECTION_GAP);
        ui.label("A Tandy Color Computer emulator");
        ui.label(egui::RichText::new("Built with Rust + egui").size(DETAIL_SIZE));
        ui.add_space(SECTION_GAP);
        ui.label(egui::RichText::new(inventory).size(DETAIL_SIZE));
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

/// The update check's outcome. Nothing before a check, nor for a failed
/// check the user did not ask for.
fn update_line(ui: &mut egui::Ui, update: &UpdateCheck) {
    match update.status() {
        Status::Idle => {}
        Status::Checking(_) => {
            ui.label(egui::RichText::new(CHECKING_TEXT).size(DETAIL_SIZE));
        }
        Status::UpToDate => {
            ui.label(egui::RichText::new(UP_TO_DATE_TEXT).size(DETAIL_SIZE));
        }
        Status::Available(release) => {
            ui.hyperlink_to(
                format!("Version {} is available", release.version),
                &release.page_url,
            );
        }
        Status::Failed(error) => {
            if update.requested() {
                ui.colored_label(ui.visuals().error_fg_color, CHECK_FAILED_TEXT)
                    .on_hover_text(error);
            }
        }
    }
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
