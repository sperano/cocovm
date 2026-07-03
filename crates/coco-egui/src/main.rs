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
//! The Machine menu can also insert/eject a cartridge ROM pak (`.rom`/`.ccc`/`.bin`);
//! the debugger panels are still TODO.

mod about;
mod audio;
mod joy;
mod kbd_help;

use std::collections::VecDeque;
use std::path::PathBuf;

use coco_core::cart::RomPak;
use coco_core::keyboard::{self as kbd, Pos};
use coco_core::{Machine, MachineConfig};
use eframe::egui;
use joy::JoystickInputs;

/// Integer scale factor for the (small) CoCo framebuffer.
const SCALE: f32 = 3.0;
/// Physical aspect the CoCo frame fills on an NTSC set (4:3). The framebuffer is
/// 288×224 (≈1.29:1); when aspect correction is on, the image is stretched
/// horizontally to this ratio so pixels are ~3% wider than tall, as on real hardware.
const TARGET_ASPECT: f32 = 4.0 / 3.0;
/// Cap on emulated fields run in one UI update: catches up after short host
/// stalls (~130 ms) but drops time beyond that instead of spiralling.
const MAX_FIELDS_PER_UPDATE: usize = 8;
/// Longest wall-clock gap credited to the emulation clock, in seconds. Gaps
/// beyond this (window drag, app hidden, debugger pause) are discarded.
const MAX_FRAME_DT: f64 = 0.25;
/// Height reserved for the top menu bar row when sizing the window.
const MENU_BAR_H: f32 = 22.0;
/// Height reserved for the toolbar row when sizing the window.
const TOOLBAR_H: f32 = 30.0;
/// Height reserved for the bottom status bar row when sizing the window.
const STATUS_BAR_H: f32 = 22.0;
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

    /// True while taps are still queued or a tap is mid hold/gap — i.e. a paste or
    /// type-ahead burst is still draining and owns the keyboard matrix.
    fn is_active(&self) -> bool {
        !self.queue.is_empty() || !matches!(self.phase, TypePhase::Idle)
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
    show_about: bool,
    aspect_correct: bool,
    /// Wall-clock instant of the previous update while running; `None` right
    /// after a pause/start so the first frame credits no elapsed time.
    last_update: Option<std::time::Instant>,
    /// Fractional emulated fields owed to the wall clock (`DESIGN.md` §4):
    /// fields run when it reaches 1, the remainder carries over. This decouples
    /// emulation speed from the host refresh rate (120 Hz displays no longer
    /// run the CoCo at double speed).
    field_debt: f64,
    /// Per-port joystick source selection (mouse/gamepad/keys) and gamepad state.
    joysticks: JoystickInputs,
    /// cpal output stream, resampler, and volume/mute state (`audio.rs`).
    audio: audio::AudioOutput,
    /// Letterboxed display rect from the last frame's `CentralPanel`, used to map
    /// pointer position to joystick axes. One frame stale (see `drive_joysticks`).
    display_rect: egui::Rect,
    /// Whether the next inserted cartridge should tie CART* to Q (auto-run at
    /// power-up). Consulted at insert time, not retroactively — see
    /// `RomPak::from_bytes`. Off suits Disk-BASIC-style paks and carts that
    /// must be started with `EXEC &HE010`.
    autostart_cart: bool,
    /// Path of the currently inserted cartridge, if any (shown in the status
    /// bar; also gates the "Eject Cartridge" menu item).
    cart_path: Option<PathBuf>,
    /// Message from the last failed cartridge load, shown in a dismissible
    /// window until acknowledged.
    cart_error: Option<String>,
}

impl CocoApp {
    fn new(_cc: &eframe::CreationContext<'_>, rom: Box<[u8]>, cart_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            machine: Machine::new(MachineConfig::default(), rom),
            texture: None,
            running: true, // boot straight to the prompt
            kb_mode: KbMode::Positional,
            type_ahead: TypeAhead::default(),
            show_kbd_help: false,
            show_about: false,
            aspect_correct: true,
            last_update: None,
            field_debt: 0.0,
            joysticks: JoystickInputs::new(),
            display_rect: egui::Rect::NOTHING,
            audio: audio::AudioOutput::new(),
            autostart_cart: true,
            cart_path: None,
            cart_error: None,
        };
        if let Some(path) = cart_path {
            app.insert_cartridge(path);
        }
        app
    }

    /// Load a ROM pak from `path` and insert it, using the current
    /// `autostart_cart` setting. Resets the machine on success (cartridge
    /// insertion is a machine-off operation on real hardware); on failure,
    /// leaves the running cartridge (if any) untouched and records the error
    /// for [`Self::cart_error`] to display.
    fn insert_cartridge(&mut self, path: PathBuf) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.cart_error = Some(format!("could not read {}: {e}", path.display()));
                return;
            }
        };
        match RomPak::from_bytes(&bytes, self.autostart_cart) {
            Ok(pak) => {
                self.machine.insert_cartridge(Box::new(pak));
                self.machine.reset();
                self.cart_path = Some(path);
            }
            Err(e) => {
                self.cart_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Eject the current cartridge and reset the machine.
    fn eject_cartridge(&mut self) {
        self.machine.eject_cartridge();
        self.machine.reset();
        self.cart_path = None;
    }

    /// Emulated fields owed for this update, from wall-clock time at the
    /// machine's field rate (60 Hz NTSC / 50 Hz PAL).
    fn fields_due(&mut self) -> usize {
        let now = std::time::Instant::now();
        let dt = match self.last_update.replace(now) {
            Some(prev) => (now - prev).as_secs_f64().min(MAX_FRAME_DT),
            None => 0.0,
        };
        self.field_debt += dt * self.machine.config.video.field_rate_hz();
        let due = (self.field_debt as usize).min(MAX_FIELDS_PER_UPDATE);
        self.field_debt = (self.field_debt - due as f64).min(1.0);
        due
    }

    fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.machine.bus.keyboard.release_all();
            self.type_ahead.clear();
        }
    }

    /// Queue a string as symbolic key taps (used by clipboard paste and, in symbolic
    /// mode, typed text). Characters with no CoCo key are skipped; `\n`/`\r` → ENTER.
    fn enqueue_text(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(entry) = kbd::char_key(c) {
                self.type_ahead.queue.push_back(entry);
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));

        // UI hotkeys (never forwarded) and clipboard paste, both keyboard-mode-agnostic.
        // egui/eframe normalises the platform paste shortcut (Cmd+V / Ctrl+V) into a
        // single Event::Paste, so this works the same on macOS, Windows, and Linux.
        for ev in &events {
            match ev {
                egui::Event::Key { key, pressed: true, repeat: false, .. } => match key {
                    egui::Key::F12 => {
                        let next = match self.kb_mode {
                            KbMode::Positional => KbMode::Symbolic,
                            KbMode::Symbolic => KbMode::Positional,
                        };
                        self.set_mode(next);
                    }
                    egui::Key::F10 => self.show_kbd_help = !self.show_kbd_help,
                    egui::Key::F9 => self.aspect_correct = !self.aspect_correct,
                    _ => {}
                },
                egui::Event::Paste(text) => self.enqueue_text(text),
                _ => {}
            }
        }

        // Symbolic mode also turns typed characters and control keys into queued taps.
        // Arrows are skipped when a joystick port is in Keys mode (see below).
        if self.kb_mode == KbMode::Symbolic {
            let joystick_keys = self.joysticks.keys_active();
            for ev in &events {
                match ev {
                    egui::Event::Text(text) => self.enqueue_text(text),
                    egui::Event::Key { key, pressed: true, .. } => {
                        if joystick_keys && is_joystick_key(*key) {
                            continue;
                        }
                        if let Some(pos) = control_key_pos(*key) {
                            self.type_ahead.queue.push_back((pos, false));
                        }
                    }
                    _ => {}
                }
            }
        }

        // While a paste / type-ahead burst is draining it owns the matrix, in either
        // mode, so replayed taps aren't clobbered by the per-frame positional writes.
        // (The taps themselves advance once per *emulated field*, in `update`.)
        if self.type_ahead.is_active() {
            return;
        }

        // Positional mode: physical keys drive the CoCo matrix directly. Arrows and
        // Z/X are skipped when a joystick port is in Keys mode, so the two input
        // paths don't fight over the same physical keys.
        if self.kb_mode == KbMode::Positional {
            let joystick_keys = self.joysticks.keys_active();
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
                    if joystick_keys && is_joystick_key(k) {
                        continue;
                    }
                    if let Some(pos) = key_to_pos(k) {
                        kb.set(pos, *pressed);
                    }
                }
            }
        }
    }

    /// Poll and apply all joystick input sources (mouse/gamepad/keys) for both
    /// ports. Called once per `update()`, before running any emulated fields, so
    /// the pot/button state a field sees is this frame's, not last frame's.
    fn drive_joysticks(&mut self, ctx: &egui::Context) {
        self.joysticks.apply(ctx, self.display_rect, &mut self.machine);
    }
}

impl eframe::App for CocoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            // Run however many fields the wall clock owes us (real-time pacing),
            // stepping type-ahead per field so paste timing is refresh-agnostic.
            for _ in 0..self.fields_due() {
                if self.type_ahead.is_active() {
                    self.type_ahead.advance(&mut self.machine.bus.keyboard);
                }
                self.machine.run_field();
            }
            let sample_rate = self.machine.audio_sample_rate();
            self.audio.push_samples(self.machine.take_audio(), sample_rate);
            ctx.request_repaint();
        } else {
            self.last_update = None;
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

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Machine", |ui| {
                    let run_label = if self.running { "Pause" } else { "Run" };
                    if ui.button(run_label).clicked() {
                        self.running = !self.running;
                        ui.close();
                    }
                    if ui.button("Reset").clicked() {
                        self.machine.reset();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Insert Cartridge…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("ROM Pak", &["rom", "ccc", "bin"])
                            .pick_file()
                        {
                            self.insert_cartridge(path);
                        }
                    }
                    let inserted = self.cart_path.is_some();
                    if ui.add_enabled(inserted, egui::Button::new("Eject Cartridge")).clicked() {
                        self.eject_cartridge();
                        ui.close();
                    }
                    ui.checkbox(&mut self.autostart_cart, "Auto-start cartridge");
                });
                ui.menu_button("Keyboard", |ui| {
                    for mode in [KbMode::Positional, KbMode::Symbolic] {
                        if ui.selectable_label(self.kb_mode == mode, mode.label()).clicked() {
                            self.set_mode(mode);
                        }
                    }
                    ui.separator();
                    if ui.button("Key layout (F10)").clicked() {
                        self.show_kbd_help = !self.show_kbd_help;
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
                });
                ui.menu_button("Joysticks", |ui| self.joysticks.menu_ui(ui));
                ui.menu_button("Sound", |ui| self.audio.menu_ui(ui));
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = !self.show_about;
                        ui.close();
                    }
                });
            });
        });

        // Toolbar: one-click access to the most frequent actions, redundant with
        // (but quicker than) the menu bar above.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let run_label = if self.running { "Pause" } else { "Run" };
                if ui.button(run_label).clicked() {
                    self.running = !self.running;
                }
                if ui.button("Reset").clicked() {
                    self.machine.reset();
                }
                ui.separator();
                if ui.button("⌨ Keys (F10)").clicked() {
                    self.show_kbd_help = !self.show_kbd_help;
                }
                ui.checkbox(&mut self.aspect_correct, "4:3 (F9)");
            });
        });

        // Status bar: read-only live state, no controls.
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(if self.running { "Running" } else { "Paused" });
                ui.separator();
                ui.label(format!("Keyboard: {} (F12)", self.kb_mode.label()));
                ui.separator();
                ui.label(format!("cycles: {}", self.machine.cpu.cycles));
                if let Some(path) = &self.cart_path {
                    ui.separator();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    ui.label(format!("Cart: {name}"));
                }
            });
        });

        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if let Some(err) = self.cart_error.clone() {
            let mut open = true;
            egui::Window::new("Cartridge Error")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(err);
                    if ui.button("OK").clicked() {
                        self.cart_error = None;
                    }
                });
            if !open {
                self.cart_error = None;
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let tex = self.texture.as_ref().unwrap();
                let tex_size = tex.size_vec2();
                // Aspect the displayed frame should have, independent of the buffer's
                // pixel dimensions: 4:3 when corrected, else the raw square-pixel aspect.
                // This keeps the frontend mode-agnostic — any renderer's buffer size fits.
                let aspect = if self.aspect_correct {
                    TARGET_ASPECT
                } else {
                    tex_size.x / tex_size.y
                };
                // Largest rect of that aspect that fits the panel, centered (letterboxed).
                let avail = ui.available_rect_before_wrap();
                let mut w = avail.width();
                let mut h = w / aspect;
                if h > avail.height() {
                    h = avail.height();
                    w = h * aspect;
                }
                let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
                let sized = egui::load::SizedTexture::new(tex.id(), rect.size());
                ui.put(rect, egui::Image::new(sized));
                // Remembered for `drive_joysticks` next frame, to map pointer
                // position to joystick axes (see the `display_rect` field doc).
                self.display_rect = rect;
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

/// Keys claimed by `joy::JoySource::Keys` (arrows for the axes, Z/X for the fire
/// buttons) once a joystick port uses that source — these stop reaching the CoCo
/// keyboard matrix so the two consumers don't fight over the same physical keys.
fn is_joystick_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::Z
            | egui::Key::X
    )
}

/// Resolve the boot ROM: the first CLI argument, else `roms/coco3.rom` at the
/// workspace root. The ROM is copyrighted and git-ignored (`./roms`).
fn load_rom() -> std::io::Result<Box<[u8]>> {
    let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom")
    });
    Ok(std::fs::read(path)?.into_boxed_slice())
}

/// Resolve an optional cartridge ROM pak: the second CLI argument, if given.
fn cart_arg() -> Option<PathBuf> {
    std::env::args().nth(2).map(PathBuf::from)
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
    let cart_path = cart_arg();
    // Size for the aspect-corrected (wider) image so it always fits; the
    // uncorrected image is narrower and simply leaves margin.
    let img_h = coco_core::video::FB_H as f32 * SCALE;
    let win_w = img_h * TARGET_ASPECT;
    let win_h = img_h + MENU_BAR_H + TOOLBAR_H + STATUS_BAR_H;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([win_w, win_h])
            .with_icon(icon),
        ..Default::default()
    };
    eframe::run_native(
        "coco-rs",
        options,
        Box::new(|cc| Ok(Box::new(CocoApp::new(cc, rom, cart_path)))),
    )
}
