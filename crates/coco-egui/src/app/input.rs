//! Host input: keyboard shortcuts the app consumes itself, and the two
//! ways host keys reach the emulated matrix (positional and symbolic).

use crate::*;

impl CocoApp {
    pub(crate) fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.machine.bus.keyboard.release_all();
            self.type_ahead.clear();
        }
    }

    /// Queue a string as symbolic key taps (used by clipboard paste and, in symbolic
    /// mode, typed text). Characters with no CoCo key are skipped; `\n`/`\r` → ENTER.
    pub(crate) fn enqueue_text(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(entry) = kbd::char_key(c) {
                self.type_ahead.queue.push_back(entry);
            }
        }
    }

    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        self.consume_app_shortcuts(ctx);

        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        self.handle_hotkeys_and_paste(&events);
        if self.kb_mode == KbMode::Symbolic {
            self.queue_symbolic_taps(&events);
        }

        // While a paste / type-ahead burst is draining it owns the matrix, in either
        // mode, so replayed taps aren't clobbered by the per-frame positional writes.
        // (The taps themselves advance once per *emulated field*, in `update`.)
        if self.type_ahead.is_active() {
            return;
        }
        if self.kb_mode == KbMode::Positional {
            self.drive_matrix_positionally(&events, mods);
        }
    }

    /// Shortcuts that open UI rather than reaching the machine. Consumed
    /// before the event snapshot `handle_input` takes, so the keypress never
    /// reaches the CoCo matrix or the symbolic type-ahead (the held modifier
    /// alone is harmless there).
    pub(crate) fn consume_app_shortcuts(&mut self, ctx: &egui::Context) {
        // COMMAND+<n> quick-loads state slot n; COMMAND+SHIFT+<n> quick-saves
        // it (`save_state.rs`).
        for slot in 0..save_state::QUICK_SLOTS {
            if ctx.input_mut(|i| i.consume_shortcut(&save_state::save_slot_shortcut(slot))) {
                self.quick_save(slot);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&save_state::load_slot_shortcut(slot))) {
                self.quick_load(slot, ctx);
            }
        }
    }

    /// UI hotkeys (never forwarded) and clipboard paste, both
    /// keyboard-mode-agnostic. egui/eframe normalises the platform paste
    /// shortcut (Cmd+V / Ctrl+V) into a single `Event::Paste`, so this works
    /// the same on macOS, Windows, and Linux.
    pub(crate) fn handle_hotkeys_and_paste(&mut self, events: &[egui::Event]) {
        for ev in events {
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
                    egui::Key::F11 => self.debugger.open = !self.debugger.open,
                    _ => {}
                },
                egui::Event::Paste(text) => self.enqueue_text(text),
                _ => {}
            }
        }
    }

    /// Symbolic mode turns typed characters and control keys into queued taps.
    /// Arrows are skipped when a joystick port is in Keys mode.
    pub(crate) fn queue_symbolic_taps(&mut self, events: &[egui::Event]) {
        let joystick_keys = self.joysticks.keys_active();
        for ev in events {
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

    /// Positional mode: physical keys drive the CoCo matrix directly. Arrows
    /// and Z/X are skipped when a joystick port is in Keys mode, so the two
    /// input paths don't fight over the same physical keys.
    pub(crate) fn drive_matrix_positionally(&mut self, events: &[egui::Event], mods: egui::Modifiers) {
        let joystick_keys = self.joysticks.keys_active();
        let kb = &mut self.machine.bus.keyboard;
        kb.set(kbd::SHIFT, mods.shift);
        kb.set(kbd::CTRL, mods.ctrl);
        kb.set(kbd::ALT, mods.alt);
        for ev in events {
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

    /// Poll and apply all joystick input sources (mouse/gamepad/keys) for both
    /// ports. Called once per `update()`, before running any emulated fields, so
    /// the pot/button state a field sees is this frame's, not last frame's.
    pub(crate) fn drive_joysticks(&mut self, ctx: &egui::Context) {
        self.joysticks.apply(ctx, self.display_rect, &mut self.machine);
    }
}
