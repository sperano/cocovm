//! `coco-egui` — eframe frontend. See `DESIGN.md` §8.
//!
//! Boots the real Super Extended Color BASIC ROM and shows the GIME/VDG text
//! output as an integer-scaled texture with a Run/Pause control. ROM file dialog,
//! keyboard input, audio, and the debugger panels are still TODO.

use std::path::PathBuf;

use coco_core::{video, Machine, MachineConfig};
use eframe::egui;

/// Integer scale factor for the (small) CoCo framebuffer.
const SCALE: f32 = 3.0;
/// Emulated video fields to run per UI repaint (≈ real time at 60 Hz refresh).
const FIELDS_PER_FRAME: usize = 1;
/// Height reserved for the top menu bar when sizing the window.
const MENU_BAR_H: f32 = 30.0;

struct CocoApp {
    machine: Machine,
    texture: Option<egui::TextureHandle>,
    running: bool,
}

impl CocoApp {
    fn new(_cc: &eframe::CreationContext<'_>, rom: Box<[u8]>) -> Self {
        Self {
            machine: Machine::new(MachineConfig::default(), rom),
            texture: None,
            running: true, // boot straight to the prompt
        }
    }
}

impl eframe::App for CocoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.running {
            for _ in 0..FIELDS_PER_FRAME {
                self.machine.run_field();
            }
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
                if ui.button("Reset").clicked() {
                    self.machine.reset();
                }
                ui.separator();
                ui.label(format!("cycles: {}", self.machine.cpu.cycles));
            });
        });

        // No margin so the window can hug the framebuffer exactly.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let tex = self.texture.as_ref().unwrap();
                let sized = egui::load::SizedTexture::new(tex.id(), tex.size_vec2() * SCALE);
                ui.image(sized);
            });
    }
}

/// Resolve the boot ROM: the first CLI argument, else `roms/coco3.rom` at the
/// workspace root. The ROM is copyrighted and git-ignored (`./roms`).
fn load_rom() -> std::io::Result<Box<[u8]>> {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom")
    });
    Ok(std::fs::read(path)?.into_boxed_slice())
}

fn main() -> eframe::Result<()> {
    let rom = match load_rom() {
        Ok(rom) => rom,
        Err(e) => {
            eprintln!("coco-egui: could not load ROM: {e}");
            eprintln!("Pass a ROM path, or place one at roms/coco3.rom.");
            std::process::exit(1);
        }
    };
    let win_w = video::FB_W as f32 * SCALE;
    let win_h = video::FB_H as f32 * SCALE + MENU_BAR_H;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([win_w, win_h]),
        ..Default::default()
    };
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(|cc| Ok(Box::new(CocoApp::new(cc, rom)))),
    )
}
