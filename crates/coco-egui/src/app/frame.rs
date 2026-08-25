//! The per-frame loop: crediting wall-clock time to emulated fields,
//! running them, and getting the resulting framebuffer onto the screen.

use crate::*;

impl CocoApp {
    /// Power-cycle the core and drop the host-side audio it already emitted.
    pub(crate) fn power_cycle(&mut self) {
        self.machine.power_cycle();
        self.audio.reset();
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
    pub(crate) fn step_emulation(&mut self, ctx: &egui::Context) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            // Routed through the debugger so an enabled breakpoint/watchpoint pauses cleanly.
            for _ in 0..self.fields_due() {
                if self.type_ahead.is_active() {
                    self.type_ahead.advance(&mut self.machine.bus.keyboard);
                }
                if !self.debugger.run_field(&mut self.machine) {
                    self.running = false;
                    break;
                }
            }
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
            ctx.request_repaint();
        } else {
            self.last_update = None;
            // Drop fields owed while paused, so resuming doesn't instantly catch up.
            self.field_debt = 0.0;
        }

        self.upload_framebuffer_texture(ctx);
    }

    /// Upload the framebuffer as `self.texture`. The only part of
    /// [`Self::step_emulation`] a suspended VM's window still runs, so its
    /// picture stays on screen without input handling.
    pub(crate) fn upload_framebuffer_texture(&mut self, ctx: &egui::Context) {
        // TV chain (B&W collapse, bandwidth limit, scanlines) — a display preference, not state.
        self.tv_frame = self.tv_frame.wrapping_add(1);
        let frame = crate::display::process(
            self.display,
            self.tv,
            self.tv_frame,
            self.machine.fb_width as usize,
            &self.machine.framebuffer,
        );
        let image =
            egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.pixels);
        // NEAREST for monitors, LINEAR for TVs; passed every `set` so switching re-filters
        // immediately.
        let options = crate::display::texture_options(self.display);
        let texture = self
            .texture
            .get_or_insert_with(|| ctx.load_texture("coco-fb", image.clone(), options));
        texture.set(image, options);
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
        // Remembered for `drive_joysticks` next frame (pointer → joystick axes, mouse fire gating).
        self.display_rect = rect;
        self.display_layer = ui.layer_id();
    }

    /// The full app window for one frame: emulation step, every menu/toolbar/
    /// dialog, then the display. `pub(crate)` so the manager can call it
    /// directly on a VM it owns.
    pub(crate) fn window_ui(&mut self, ctx: &egui::Context) {
        self.step_emulation(ctx);
        self.draw_chrome(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| self.draw_display(ui));
    }
}
