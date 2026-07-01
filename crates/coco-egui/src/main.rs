//! `coco-egui` — eframe frontend. See `DESIGN.md` §8.
//!
//! STATUS: skeleton. Shows the core's framebuffer as a texture with a Run/Pause
//! control. ROM loading, input, audio, and the debugger panels are TODO.

use coco_core::{Machine, MachineConfig};
use eframe::egui;

const ROM_SIZE: usize = 32 * 1024;

struct CocoApp {
    machine: Machine,
    texture: Option<egui::TextureHandle>,
    running: bool,
}

impl CocoApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // No ROM yet — placeholder. Loading the real Super Extended Color BASIC
        // ROM is a menu action (`DESIGN.md` §8, milestone 2).
        let rom = vec![0u8; ROM_SIZE].into_boxed_slice();
        Self {
            machine: Machine::new(MachineConfig::default(), rom),
            texture: None,
            running: false,
        }
    }
}

impl eframe::App for CocoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.running {
            self.machine.run_field();
            ctx.request_repaint();
        }

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [
                self.machine.fb_width as usize,
                self.machine.fb_height as usize,
            ],
            &self.machine.framebuffer,
        );
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture("coco-fb", image.clone(), egui::TextureOptions::NEAREST)
        });
        texture.set(image, egui::TextureOptions::NEAREST);

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let label = if self.running { "Pause" } else { "Run" };
                if ui.button(label).clicked() {
                    self.running = !self.running;
                }
                ui.separator();
                ui.label(format!("cycles: {}", self.machine.cpu.cycles));
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let tex = self.texture.as_ref().unwrap();
            let sized = egui::load::SizedTexture::new(tex.id(), tex.size_vec2());
            ui.image(sized);
        });
    }
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "coco-rs",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(CocoApp::new(cc)))),
    )
}
