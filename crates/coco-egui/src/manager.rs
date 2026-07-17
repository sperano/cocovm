//! The CocoVM manager window: the VirtualBox/Parallels-style main window a
//! bare `coco` (no CLI arguments) opens instead of booting a machine
//! directly. Toolbar across the top, machine list down the left (empty for
//! now), and — while no machine is selected — a random photo asset filling
//! the right pane.
//!
//! Everything here is scaffolding for the manager flow: the toolbar buttons
//! are inert, and the list has no rows yet. The direct-boot emulator
//! (`CocoApp`) is untouched and still serves every CLI invocation with
//! arguments.

use eframe::egui;

use crate::photo_view::{self, Photo};

/// Manager window size at first open.
const WINDOW_SIZE: [f32; 2] = [1080.0, 720.0];

/// Machine-list panel: width at first open and the draggable divider's range.
const LIST_DEFAULT_WIDTH: f32 = 260.0;
const LIST_MIN_WIDTH: f32 = 160.0;
const LIST_MAX_WIDTH: f32 = 520.0;

pub struct ManagerApp {
    /// Decoded photo pending its first-frame texture upload.
    photo: Option<Photo>,
    /// The uploaded photo texture, once a frame has run.
    photo_texture: Option<egui::TextureHandle>,
}

impl ManagerApp {
    /// `photo` is injected (rather than loaded here) so tests can construct
    /// the manager without touching the user's real asset directory.
    pub fn new(photo: Option<Photo>) -> Self {
        Self {
            photo,
            photo_texture: None,
        }
    }
}

impl eframe::App for ManagerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(photo) = self.photo.take() {
            self.photo_texture =
                Some(ctx.load_texture(&photo.title, photo.pixels, egui::TextureOptions::LINEAR));
        }

        // Toolbar: the manager actions. All inert scaffolding for now.
        egui::TopBottomPanel::top("manager_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let _ = ui.button("New…");
                let _ = ui.button("Settings");
                let _ = ui.button("Help");
            });
        });

        // Machine list: one row per configured instance eventually; empty
        // scaffolding for now. `resizable` gives the draggable divider
        // between the list and the photo pane.
        egui::SidePanel::left("manager_machine_list")
            .resizable(true)
            .default_width(LIST_DEFAULT_WIDTH)
            .width_range(LIST_MIN_WIDTH..=LIST_MAX_WIDTH)
            .show(ctx, |_ui| {});

        // Right pane: with no machine selected (always, for now), a random
        // photo asset, centered and scaled to fit.
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.photo_texture {
                ui.centered_and_justified(|ui| {
                    ui.add(
                        egui::Image::new(texture)
                            .max_size(ui.available_size())
                            .maintain_aspect_ratio(true),
                    );
                });
            }
        });
    }
}

/// Open the manager as the application's main window (blocks until close,
/// like `eframe::run_native` everywhere else).
pub fn run() -> eframe::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(WINDOW_SIZE)
            .with_icon(icon)
            .with_title("CocoVM"),
        ..Default::default()
    };
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(|_cc| Ok(Box::new(ManagerApp::new(photo_view::random())))),
    )
}
