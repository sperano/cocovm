//! The per-frame loop: crediting wall-clock time to emulated fields,
//! running them, and getting the resulting framebuffer onto the screen.

use std::time::Duration;

use crate::*;

/// `Some(BACKGROUND_REPAINT_INTERVAL)` when every viewport reports unfocused;
/// `None` (full rate) if any is focused, unknown (`None`), or there are none.
pub(crate) fn background_repaint_delay(
    focus: impl IntoIterator<Item = Option<bool>>,
) -> Option<Duration> {
    let mut focus = focus.into_iter().peekable();
    focus.peek()?;
    let any_focused = focus.any(|f| f.unwrap_or(true));
    (!any_focused).then_some(BACKGROUND_REPAINT_INTERVAL)
}

impl CocoApp {
    /// Power-cycle the core and drop the host-side audio it already emitted.
    pub(crate) fn power_cycle(&mut self) {
        self.machine.power_cycle();
        self.reset_audio();
    }

    /// Drop queued host audio and the cushion accounting that went with it.
    pub(crate) fn reset_audio(&mut self) {
        self.audio.reset();
        self.audio_cushion_fields = 0;
    }

    /// Entering the throttle runs one interval of fields ahead so the ring never
    /// drains between wake-ups; leaving it owes those fields back, so emulation
    /// idles while the cushion plays out and realigns with the wall clock.
    fn adjust_audio_cushion(&mut self, repaint_delay: Option<Duration>) {
        match repaint_delay {
            Some(delay) if self.audio_cushion_fields == 0 => {
                let fields = (delay.as_secs_f64() * self.machine.config.video.field_rate_hz())
                    .ceil() as usize;
                self.run_fields(fields);
                self.audio_cushion_fields = fields;
            }
            None if self.audio_cushion_fields > 0 => {
                self.field_debt -= self.audio_cushion_fields as f64;
                self.audio_cushion_fields = 0;
            }
            _ => {}
        }
    }

    /// The cushion adjustment, then the fields the wall clock owes — unless a
    /// breakpoint inside the cushion run already paused the machine.
    fn run_emulation_fields(&mut self, repaint_delay: Option<Duration>) {
        self.adjust_audio_cushion(repaint_delay);
        if self.running {
            let due = self.fields_due();
            self.run_fields(due);
        }
    }

    /// Run `n` fields through the debugger, stopping (and clearing `running`)
    /// on a breakpoint/watchpoint.
    fn run_fields(&mut self, n: usize) {
        for _ in 0..n {
            let _perf = crate::perf::span(crate::perf::Stage::FieldExecution);
            if self.type_ahead.is_active() {
                self.type_ahead.advance(&mut self.machine.bus.keyboard);
            }
            if self.remote_type_ahead.is_active() {
                self.remote_type_ahead
                    .advance(&mut self.machine.bus.keyboard);
            }
            self.advance_remote_hold();
            if self.debugger.run_field(&mut self.machine) {
                self.fields_run += 1;
            } else {
                self.running = false;
                break;
            }
        }
    }

    /// Decrement an in-progress `press_keys` hold, releasing every held
    /// position and clearing [`Self::remote_held`] once it reaches 0. Runs
    /// before the breakpoint check so a hold due to release this field isn't
    /// skipped by a breakpoint hit on the same field.
    fn advance_remote_hold(&mut self) {
        let Some(hold) = self.remote_held.as_mut() else {
            return;
        };
        if hold.fields_left == 0 {
            for &pos in &hold.keys {
                self.machine.bus.keyboard.set(pos, false);
            }
            self.remote_held = None;
        } else {
            // Re-asserted every field so a focus-loss `release_all` can't cut
            // the hold short.
            for &pos in &hold.keys {
                self.machine.bus.keyboard.set(pos, true);
            }
            hold.fields_left -= 1;
        }
    }

    /// Emulated fields owed for this update, from wall-clock time at the machine's
    /// field rate. Also accumulates [`Self::total_runtime`], clamped by
    /// [`MAX_FRAME_DT`] so a host stall can't burst the catch-up.
    pub(crate) fn fields_due(&mut self) -> usize {
        let now = std::time::Instant::now();
        let elapsed = self
            .last_update
            .replace(now)
            .map_or(std::time::Duration::ZERO, |prev| now - prev);
        let clamped = elapsed.min(std::time::Duration::from_secs_f64(MAX_FRAME_DT));
        self.total_runtime += clamped;
        let dt = clamped.as_secs_f64();
        self.field_debt += dt * self.machine.config.video.field_rate_hz();
        let due = (self.field_debt as usize).min(MAX_FIELDS_PER_UPDATE);
        self.field_debt = (self.field_debt - due as f64).min(1.0);
        due
    }

    /// Advance emulation for one host frame: input, joysticks, the field loop,
    /// audio, and the framebuffer upload. Runs before any chrome is drawn.
    /// `repaint_delay` is the caller's [`background_repaint_delay`] decision.
    pub(crate) fn step_emulation(&mut self, ctx: &egui::Context, repaint_delay: Option<Duration>) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            self.run_emulation_fields(repaint_delay);
            // Only the save half runs here — re-finalizing would fold a same-field
            // capture into the already-landed recording, discarding its leader.
            if self.machine.bus.cassette.take_recording_landed()
                && let Err(e) = self.save_tape_bytes()
            {
                self.cart_error = Some(e);
            }
            let sample_rate = self.machine.audio_sample_rate();
            self.audio
                .push_samples(self.machine.take_audio(), sample_rate);
            match repaint_delay {
                Some(delay) => ctx.request_repaint_after(delay),
                None => ctx.request_repaint(),
            }
        } else {
            self.last_update = None;
            // Drop fields owed while paused, so resuming doesn't instantly catch up.
            self.field_debt = 0.0;
            self.audio_cushion_fields = 0;
        }

        self.upload_framebuffer_texture(ctx);
    }

    /// Upload the framebuffer as `self.texture`. This is the only part of
    /// [`Self::step_emulation`] that a suspended VM's window still runs, so its
    /// picture stays on screen without input handling.
    pub(crate) fn upload_framebuffer_texture(&mut self, ctx: &egui::Context) {
        self.present_framebuffer(ctx, std::time::Instant::now());
    }

    fn present_framebuffer(&mut self, ctx: &egui::Context, now: std::time::Instant) {
        let background_delay =
            ctx.input(|i| background_repaint_delay(i.raw.viewports.values().map(|v| v.focused)));
        if let Some(delay) =
            self.presentation
                .repaint_after(self.display, self.tv, now, background_delay)
        {
            ctx.request_repaint_after(delay);
        }
        if self.texture.is_none() {
            self.presentation.invalidate();
        }
        let Some(image) = self.presentation.prepare(
            self.display,
            self.tv,
            self.machine.fb_width as usize,
            &self.machine.framebuffer,
            now,
        ) else {
            return;
        };
        let _enqueue = crate::perf::span(crate::perf::Stage::TextureEnqueue);
        let options = crate::display::texture_options(self.display);
        crate::perf::texture_enqueue(image.pixels.len() * coco_core::video::BYTES_PER_PIXEL);
        if let Some(texture) = &mut self.texture {
            texture.set(image, options);
        } else {
            self.texture = Some(ctx.load_texture("coco-fb", image, options));
        }
    }

    /// The CoCo display: the letterboxed, optionally aspect-corrected framebuffer
    /// texture. Requires [`Self::step_emulation`] to have already run this frame.
    pub(crate) fn draw_display(&mut self, ui: &mut egui::Ui) {
        let tex = self.texture.as_ref().unwrap();
        let tex_size = tex.size_vec2();
        // 4:3 when corrected, else the raw square-pixel aspect.
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
        let uv = crate::display::texture_uv(self.display, self.tv);
        ui.put(rect, egui::Image::new(sized).uv(uv));
        // Remembered for `drive_joysticks` next frame (pointer → joystick axes,
        // mouse fire gating).
        self.display_rect = rect;
        self.display_layer = ui.layer_id();
    }

    /// The full app window for one frame: emulation step, every menu/toolbar/
    /// dialog, then the display. `pub(crate)` so the manager can call it
    /// directly on a VM it owns.
    pub(crate) fn window_ui(&mut self, ctx: &egui::Context, repaint_delay: Option<Duration>) {
        let _perf = crate::perf::span(crate::perf::Stage::VmUiUpdate);
        if self.suspended {
            self.suspended_window_ui(ctx);
        } else {
            self.drew_suspended = false;
            self.step_emulation(ctx, repaint_delay);
            self.draw_chrome(ctx);
            self.display_panel(ctx);
        }
    }

    /// [`Self::window_ui`] while suspended: the frozen frame under the same
    /// chrome, drawn read-only (`self.suspended`), with no input handling or
    /// emulation step. On the first suspended frame any popup left open from
    /// the running window is closed — only then, since immediate viewports
    /// share popup memory with the manager window.
    fn suspended_window_ui(&mut self, ctx: &egui::Context) {
        if !self.drew_suspended {
            egui::Popup::close_all(ctx);
            self.drew_suspended = true;
        }
        self.upload_framebuffer_texture(ctx);
        self.draw_chrome(ctx);
        self.display_panel(ctx);
    }

    fn display_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| self.draw_display(ui));
    }
}

#[cfg(test)]
#[path = "frame_test.rs"]
mod tests;

#[cfg(test)]
#[path = "presentation_integration_test.rs"]
mod presentation_tests;
