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
        // The F-key hotkeys toggle UI, never reach the machine, and text
        // widgets don't consume F-keys — so they stay live even while one
        // is focused.
        self.handle_hotkeys(&events);

        // `wants_keyboard_input()` reports "some widget currently holds egui
        // focus", not "a text widget is focused": a click only ever focuses
        // a text-editing widget, but Tab can focus any clickable one (every
        // `Sense::click` widget is focusable), and Escape clears focus
        // again. The case this gate exists for is a focused text widget
        // (the tape menu's seek field, the RS-232 address, a debugger goto
        // box…) owning the keyboard: none of its keystrokes may reach the
        // CoCo matrix or the symbolic type-ahead, and a paste must land in
        // the widget, not the machine. Releasing the matrix (idempotent)
        // also unsticks any key that was held down when the widget grabbed
        // focus, since its release event will never get here. A stray
        // Tab-focused button gates input the same way, but Escape releases
        // it.
        if ctx.wants_keyboard_input() {
            self.machine.bus.keyboard.release_all();
            return;
        }

        self.handle_paste(&events);
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
        // ⌘N is the MANAGER's new-machine shortcut and means nothing in a
        // VM window — but it's still consumed here, as a deliberate no-op,
        // so a user hitting it out of habit doesn't type an `N` into the
        // running machine via the positional matrix (which forwards keys
        // regardless of the COMMAND modifier).
        let _ = ctx.input_mut(|i| i.consume_shortcut(&new_vm::NEW_MACHINE_SHORTCUT));
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
        // ⌘D toggles the debugger (also the toolbar's Debug tile). Consumed
        // here — before this frame's widgets run — so it stays live even
        // while a text widget in the VM window owns egui focus (the tape
        // seek field, the RS-232 address), and the consumed `D` never
        // reaches the CoCo matrix. This only sees the VM viewport's input;
        // ⌘D typed in the debugger's own native window is consumed by
        // `DebuggerPanel::windows_ui` instead.
        if ctx.input_mut(|i| i.consume_shortcut(&debugger::DEBUGGER_SHORTCUT)) {
            self.debugger.toggle();
        }
    }

    /// UI hotkeys, never forwarded to the machine and (unlike the paste
    /// path) not gated on text-widget focus — see `handle_input`.
    fn handle_hotkeys(&mut self, events: &[egui::Event]) {
        for ev in events {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                ..
            } = ev
            {
                match key {
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
                }
            }
        }
    }

    /// Clipboard paste into the machine, keyboard-mode-agnostic. egui/eframe
    /// normalises the platform paste shortcut (Cmd+V / Ctrl+V) into a single
    /// `Event::Paste`, so this works the same on macOS, Windows, and Linux.
    fn handle_paste(&mut self, events: &[egui::Event]) {
        for ev in events {
            if let egui::Event::Paste(text) = ev {
                self.enqueue_text(text);
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
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
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
    pub(crate) fn drive_matrix_positionally(
        &mut self,
        events: &[egui::Event],
        mods: egui::Modifiers,
    ) {
        let joystick_keys = self.joysticks.keys_active();
        let kb = &mut self.machine.bus.keyboard;
        kb.set(kbd::SHIFT, mods.shift);
        kb.set(kbd::CTRL, mods.ctrl);
        kb.set(kbd::ALT, mods.alt);
        for ev in events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed,
                ..
            } = ev
            {
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
        let active_rect = self.active_screen_rect();
        self.joysticks.apply(
            ctx,
            self.display_rect,
            active_rect,
            self.display_layer,
            &mut self.machine,
        );
    }

    /// The active (non-border) picture's ON-SCREEN rect: [`Machine::active_rect`]'s
    /// framebuffer-pixel geometry, scaled into `self.display_rect` by
    /// [`scale_active_rect`] — mouse-as-joystick tracks the active picture,
    /// not the full bordered display.
    ///
    /// The fb dims read here are consistent with the texture behind the
    /// one-frame-stale `display_rect`: `drive_joysticks` runs at the top of
    /// `update`, BEFORE this frame's `step_emulation` re-renders and
    /// re-uploads — so `fb_width`/`fb_height` still describe the frame
    /// `draw_display` fit `display_rect` to. A reorder that moves joystick
    /// polling after `step_emulation` would quietly break that pairing.
    fn active_screen_rect(&self) -> egui::Rect {
        scale_active_rect(
            self.machine.active_rect(),
            self.machine.fb_width,
            self.machine.fb_height,
            self.display_rect,
        )
    }
}

/// Scale a framebuffer-pixel [`coco_core::ActiveRect`] into the on-screen
/// `display` rect. Correct for ANY per-axis linear stretch between the two:
/// the whole framebuffer is stretched over the whole `display` rect with no
/// crop or interior letterbox (`CocoApp::draw_display`), and `sx`/`sy` are
/// computed independently — so the TV chain's height-only scanline doubling
/// (`display::expand_scanlines`) and `draw_display`'s 4:3 aspect fit are
/// both absorbed. What it can NOT survive is a crop or non-linear warp
/// between framebuffer and screen; none exists today. Falls back to
/// `display` itself when the framebuffer has zero width/height — shouldn't
/// happen, but avoids dividing by zero.
fn scale_active_rect(
    active: coco_core::ActiveRect,
    fb_w: u32,
    fb_h: u32,
    display: egui::Rect,
) -> egui::Rect {
    if fb_w == 0 || fb_h == 0 {
        return display;
    }
    let sx = display.width() / fb_w as f32;
    let sy = display.height() / fb_h as f32;
    egui::Rect::from_min_size(
        display.left_top() + egui::vec2(active.x as f32 * sx, active.y as f32 * sy),
        egui::vec2(active.width as f32 * sx, active.height as f32 * sy),
    )
}

#[cfg(test)]
#[path = "input_test.rs"]
mod tests;
