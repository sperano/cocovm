//! `coco-egui` — eframe frontend. See `DESIGN.md` §8.
//!
//! Boots the real Super Extended Color BASIC ROM, shows the GIME/VDG output as an
//! integer-scaled texture, and feeds host keyboard input into the CoCo matrix in
//! one of two modes (toggle with F12):
//!
//! - Positional — physical key → CoCo matrix position (CoCo applies its own shift
//!   semantics, like MAME). The default.
//! - Symbolic — the character you type is injected via the CoCo keys that produce it.
//!
//! ROM file dialog, audio, and the debugger panels are still TODO.

mod kbd_help;

use std::collections::VecDeque;
use std::path::PathBuf;

use coco_core::keyboard::{self as kbd, Pos};
use coco_core::{Machine, MachineConfig};
use eframe::egui;

/// Integer scale factor for the (small) CoCo framebuffer.
const SCALE: f32 = 3.0;
/// Emulated video fields to run per UI repaint (≈ real time at 60 Hz refresh).
const FIELDS_PER_FRAME: usize = 1;
/// Height reserved for the top menu bar when sizing the window.
const MENU_BAR_H: f32 = 30.0;
/// Symbolic-mode key timing, in fields: hold a synthesized key then release.
const TYPE_HOLD_FIELDS: u8 = 2;
const TYPE_GAP_FIELDS: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum KbMode {
    Positional,
    Symbolic,
}

impl KbMode {
    fn label(self) -> &'static str {
        match self {
            KbMode::Positional => "Positional",
            KbMode::Symbolic => "Symbolic",
        }
    }
}

/// Symbolic-mode type-ahead: replays queued (key, shift) taps with hold/gap timing
/// so the ROM's 60 Hz keyboard scan registers each one.
#[derive(Default)]
struct TypeAhead {
    queue: VecDeque<(Pos, bool)>,
    phase: TypePhase,
    current: (Pos, bool),
}

#[derive(Default, Clone, Copy)]
enum TypePhase {
    #[default]
    Idle,
    Hold(u8),
    Gap(u8),
}

impl TypeAhead {
    fn clear(&mut self) {
        self.queue.clear();
        self.phase = TypePhase::Idle;
    }

    /// Advance one field, driving the CoCo matrix for the current tap.
    fn advance(&mut self, kb: &mut kbd::Keyboard) {
        match self.phase {
            TypePhase::Idle => {
                if let Some(entry) = self.queue.pop_front() {
                    self.current = entry;
                    kb.set(entry.0, true);
                    if entry.1 {
                        kb.set(kbd::SHIFT, true);
                    }
                    self.phase = TypePhase::Hold(TYPE_HOLD_FIELDS);
                }
            }
            TypePhase::Hold(0) => {
                kb.set(self.current.0, false);
                kb.set(kbd::SHIFT, false);
                self.phase = TypePhase::Gap(TYPE_GAP_FIELDS);
            }
            TypePhase::Hold(n) => self.phase = TypePhase::Hold(n - 1),
            TypePhase::Gap(0) => self.phase = TypePhase::Idle,
            TypePhase::Gap(n) => self.phase = TypePhase::Gap(n - 1),
        }
    }
}

struct CocoApp {
    machine: Machine,
    texture: Option<egui::TextureHandle>,
    running: bool,
    kb_mode: KbMode,
    type_ahead: TypeAhead,
    show_kbd_help: bool,
}

impl CocoApp {
    fn new(_cc: &eframe::CreationContext<'_>, rom: Box<[u8]>) -> Self {
        Self {
            machine: Machine::new(MachineConfig::default(), rom),
            texture: None,
            running: true, // boot straight to the prompt
            kb_mode: KbMode::Positional,
            type_ahead: TypeAhead::default(),
            show_kbd_help: false,
        }
    }

    fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.machine.bus.keyboard.release_all();
            self.type_ahead.clear();
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));

        // F10/F12 are UI hotkeys, never forwarded to the CoCo.
        for ev in &events {
            if let egui::Event::Key { key, pressed: true, repeat: false, .. } = ev {
                match key {
                    egui::Key::F12 => {
                        let next = match self.kb_mode {
                            KbMode::Positional => KbMode::Symbolic,
                            KbMode::Symbolic => KbMode::Positional,
                        };
                        self.set_mode(next);
                    }
                    egui::Key::F10 => self.show_kbd_help = !self.show_kbd_help,
                    _ => {}
                }
            }
        }

        match self.kb_mode {
            KbMode::Positional => {
                let kb = &mut self.machine.bus.keyboard;
                kb.set(kbd::SHIFT, mods.shift);
                kb.set(kbd::CTRL, mods.ctrl);
                kb.set(kbd::ALT, mods.alt);
                for ev in &events {
                    if let egui::Event::Key { key, physical_key, pressed, .. } = ev {
                        let k = physical_key.unwrap_or(*key);
                        if k == egui::Key::F12 {
                            continue;
                        }
                        if let Some(pos) = key_to_pos(k) {
                            kb.set(pos, *pressed);
                        }
                    }
                }
            }
            KbMode::Symbolic => {
                for ev in &events {
                    match ev {
                        egui::Event::Text(text) => {
                            for c in text.chars() {
                                if let Some(entry) = kbd::char_key(c) {
                                    self.type_ahead.queue.push_back(entry);
                                }
                            }
                        }
                        egui::Event::Key { key, pressed: true, .. } => {
                            if let Some(pos) = control_key_pos(*key) {
                                self.type_ahead.queue.push_back((pos, false));
                            }
                        }
                        _ => {}
                    }
                }
                self.type_ahead.advance(&mut self.machine.bus.keyboard);
            }
        }
    }
}

impl eframe::App for CocoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);

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
                ui.label(format!("Keyboard: {} (F12)", self.kb_mode.label()));
                if ui.button("⌨ Keys (F10)").clicked() {
                    self.show_kbd_help = !self.show_kbd_help;
                }
                ui.separator();
                ui.label(format!("cycles: {}", self.machine.cpu.cycles));
            });
        });

        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let tex = self.texture.as_ref().unwrap();
                let sized = egui::load::SizedTexture::new(tex.id(), tex.size_vec2() * SCALE);
                ui.image(sized);
            });
    }
}

/// Positional map: host physical key → CoCo matrix position (MAME's layout).
fn key_to_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        // Letters: @ A..Z run linearly from (0,0).
        K::A => (0, 1), K::B => (0, 2), K::C => (0, 3), K::D => (0, 4),
        K::E => (0, 5), K::F => (0, 6), K::G => (0, 7),
        K::H => (1, 0), K::I => (1, 1), K::J => (1, 2), K::K => (1, 3),
        K::L => (1, 4), K::M => (1, 5), K::N => (1, 6), K::O => (1, 7),
        K::P => (2, 0), K::Q => (2, 1), K::R => (2, 2), K::S => (2, 3),
        K::T => (2, 4), K::U => (2, 5), K::V => (2, 6), K::W => (2, 7),
        K::X => (3, 0), K::Y => (3, 1), K::Z => (3, 2),
        // Digits.
        K::Num0 => (4, 0), K::Num1 => (4, 1), K::Num2 => (4, 2), K::Num3 => (4, 3),
        K::Num4 => (4, 4), K::Num5 => (4, 5), K::Num6 => (4, 6), K::Num7 => (4, 7),
        K::Num8 => (5, 0), K::Num9 => (5, 1),
        // Punctuation (host physical key → CoCo key at that position, per MAME).
        K::Minus => (5, 2),      // CoCo ':'
        K::Semicolon => (5, 3),  // CoCo ';'
        K::Comma => (5, 4),      // CoCo ','
        K::Equals => (5, 5),     // CoCo '-'
        K::Period => (5, 6),     // CoCo '.'
        K::Slash => (5, 7),      // CoCo '/'
        K::OpenBracket => kbd::AT,
        // Movement / control.
        K::Space => kbd::SPACE,
        K::Enter => kbd::ENTER,
        K::Backspace => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowLeft => kbd::LEFT,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
}

/// Control keys that symbolic mode still routes positionally (they produce no text).
fn control_key_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        K::Enter => kbd::ENTER,
        K::Backspace | K::ArrowLeft => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
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
    let win_w = coco_core::video::FB_W as f32 * SCALE;
    let win_h = coco_core::video::FB_H as f32 * SCALE + MENU_BAR_H;
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
