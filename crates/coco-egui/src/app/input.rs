//! Host input: keyboard shortcuts the app consumes itself, and the two
//! ways host keys reach the emulated matrix (positional and symbolic).

use crate::*;

/// Whether the native viewport can deliver complete keyboard press/release pairs.
/// Check both the retained focus flag and this frame's event so synthetic and
/// backend-provided focus transitions follow the same path.
pub(crate) fn has_keyboard_focus(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.focused
            && !input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
    })
}

impl CocoApp {
    pub(crate) fn set_mode(&mut self, mode: KbMode) {
        if mode != self.kb_mode {
            self.kb_mode = mode;
            self.release_keyboard_state();
        }
    }

    /// Releases the host-driven half of the matrix on focus loss/text-widget
    /// takeover. Deliberately leaves `remote_type_ahead`/`remote_held`
    /// alone — a control-protocol driver's session is unaffected by host
    /// focus, since it is driven from `run_fields` (`app/frame.rs`), not
    /// from here.
    fn release_keyboard_state(&mut self) {
        self.machine.bus.keyboard.release_all();
        self.type_ahead.clear();
        self.keyboard_modifiers = typeahead::KeyModifiers::default();
    }

    /// Queue a string as symbolic key taps (used by clipboard paste and, in symbolic
    /// mode, typed text). Characters with no CoCo key are skipped; `\n`/`\r` → ENTER.
    pub(crate) fn enqueue_text(&mut self, text: &str) {
        for c in text.chars() {
            if let Some(entry) = kbd::char_key(c) {
                self.type_ahead.queue.push_back(entry.into());
            }
        }
    }

    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        // A lost-focus frame can omit releases and retain stale modifiers. Keep this gate
        // active until focus returns so neither positional input nor typeahead can reassert
        // the matrix while the viewport is unfocused.
        if !has_keyboard_focus(ctx) {
            self.release_keyboard_state();
            return;
        }

        self.consume_app_shortcuts(ctx);

        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));

        // A focused text widget (or Tab-focused button) must own the keyboard; releasing
        // the matrix also unsticks any key that was held when it grabbed focus.
        if ctx.wants_keyboard_input() {
            self.release_keyboard_state();
            return;
        }

        self.handle_paste(&events);
        if self.kb_mode == KbMode::Symbolic {
            self.queue_symbolic_taps(&events);
        }

        // While a paste/type-ahead burst drains, it owns the matrix so replayed taps
        // aren't clobbered by the per-frame positional writes. A remote `type_text`/
        // `press_keys` session must win the same way, so a focused VM window's
        // positional pass doesn't clobber a key it's mid-hold on.
        if self.type_ahead.is_active()
            || self.remote_type_ahead.is_active()
            || self.remote_held.is_some()
        {
            return;
        }
        if self.kb_mode == KbMode::Positional {
            self.drive_matrix_positionally(&events, mods);
        }
    }

    /// Shortcuts that open UI rather than reaching the machine. Consumed before
    /// `handle_input`'s event snapshot, so the keypress never reaches the CoCo matrix,
    /// and before its text-widget gate, so they stay live while a text field has focus.
    pub(crate) fn consume_app_shortcuts(&mut self, ctx: &egui::Context) {
        // The hotkeys match exactly, so they go before the state chords, which ignore an
        // extra Shift/Alt and would otherwise take e.g. a Cmd+Alt+1 hotkey.
        let hotkeys = self.hotkeys;
        // Consumed here as a deliberate no-op, so it doesn't type into the machine.
        let _ = ctx.input_mut(|i| hotkeys.new_machine.consume(i));
        if ctx.input_mut(|i| hotkeys.key_layout.consume(i)) {
            self.show_kbd_help = !self.show_kbd_help;
        }
        if ctx.input_mut(|i| hotkeys.keyboard_mode.consume(i)) {
            self.set_mode(match self.kb_mode {
                KbMode::Positional => KbMode::Symbolic,
                KbMode::Symbolic => KbMode::Positional,
            });
        }
        // COMMAND+<n> quick-loads State n; COMMAND+SHIFT+<n> quick-saves it (States 1-3 only).
        let consume = |shortcut: Option<egui::KeyboardShortcut>| {
            shortcut.is_some_and(|s| ctx.input_mut(|i| i.consume_shortcut(&s)))
        };
        for slot in 0..save_state::QUICK_SLOTS {
            if consume(save_state::save_slot_shortcut(slot)) {
                self.quick_save(slot);
            }
            if consume(save_state::load_slot_shortcut(slot)) {
                self.quick_load_shortcut(slot, ctx);
            }
        }
        #[cfg(feature = "debug-ui")]
        {
            if ctx.input_mut(|i| hotkeys.debugger.consume(i)) {
                self.debugger.toggle();
            }
        }
    }

    /// Clipboard paste into the machine, keyboard-mode-agnostic. egui normalises the
    /// platform paste shortcut into a single `Event::Paste`.
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
                        self.type_ahead.queue.push_back((pos, false).into());
                    }
                }
                _ => {}
            }
        }
    }

    /// Positional mode: physical keys drive the CoCo matrix directly. Arrows and Z/X
    /// are skipped when a joystick port is in Keys mode.
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
                if joystick_keys && is_joystick_key(k) {
                    continue;
                }
                if let Some(pos) = key_to_pos(k) {
                    kb.set(pos, *pressed);
                }
            }
        }
    }

    /// Poll and apply all joystick input sources (mouse/gamepad/keys) for both ports,
    /// then let a control-protocol `joystick` override win over whatever the host
    /// source just wrote (including an unassigned port's own recenter).
    /// Called before running any emulated fields, so a field sees this frame's state.
    pub(crate) fn drive_joysticks(&mut self, ctx: &egui::Context) {
        let active_rect = self.active_screen_rect();
        self.joysticks.apply(
            ctx,
            self.display_rect,
            active_rect,
            self.display_layer,
            &mut self.machine,
        );
        for port in [coco_core::joystick::LEFT, coco_core::joystick::RIGHT] {
            let Some(remote) = &self.remote_joy[port] else {
                continue;
            };
            self.machine
                .bus
                .joysticks
                .set_axis(port, coco_core::joystick::AXIS_X, remote.x);
            self.machine
                .bus
                .joysticks
                .set_axis(port, coco_core::joystick::AXIS_Y, remote.y);
            self.machine
                .bus
                .joysticks
                .set_button(port, 0, remote.buttons[0]);
            self.machine
                .bus
                .joysticks
                .set_button(port, 1, remote.buttons[1]);
            self.joysticks.in_use[port] = true;
        }
    }

    /// The active (non-border) picture's on-screen rect, scaled from
    /// [`Machine::active_rect`]. Must run before `step_emulation` re-renders, or
    /// `fb_width`/`fb_height` desync from the one-frame-stale `display_rect`.
    fn active_screen_rect(&self) -> egui::Rect {
        let uv = crate::display::texture_uv(self.display, self.tv);
        scale_active_rect(
            self.machine.active_rect(),
            self.machine.fb_width,
            self.machine.fb_height,
            self.display_rect,
            uv,
        )
    }
}

/// Scale a framebuffer-pixel [`coco_core::ActiveRect`] into the on-screen
/// `display` rect through the normalized source `uv` crop. Correct for any
/// per-axis linear stretch; falls back to `display` for degenerate inputs.
fn scale_active_rect(
    active: coco_core::ActiveRect,
    fb_w: u32,
    fb_h: u32,
    display: egui::Rect,
    uv: egui::Rect,
) -> egui::Rect {
    if fb_w == 0 || fb_h == 0 || uv.width() <= 0.0 || uv.height() <= 0.0 {
        return display;
    }
    let source_size = egui::vec2(fb_w as f32 * uv.width(), fb_h as f32 * uv.height());
    let source_origin = egui::pos2(fb_w as f32 * uv.min.x, fb_h as f32 * uv.min.y);
    let sx = display.width() / source_size.x;
    let sy = display.height() / source_size.y;
    egui::Rect::from_min_size(
        display.left_top()
            + egui::vec2(
                (active.x as f32 - source_origin.x) * sx,
                (active.y as f32 - source_origin.y) * sy,
            ),
        egui::vec2(active.width as f32 * sx, active.height as f32 * sy),
    )
}

#[cfg(test)]
#[path = "input_test.rs"]
mod tests;
