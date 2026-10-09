//! The per-frame loop: crediting wall-clock time to emulated fields,
//! running them, and getting the resulting framebuffer onto the screen.

use std::time::Duration;

use super::scheduling;
use crate::*;

/// Scrim over the frozen frame of a suspended display.
pub(crate) const SUSPENDED_SCRIM: egui::Color32 = egui::Color32::from_black_alpha(140);
/// Text of the suspended display's centered marker.
const SUSPENDED_OVERLAY_TEXT: &str = "Suspended";
/// Marker text height as a fraction of the display rect, so it scales with the window.
const SUSPENDED_TEXT_HEIGHT_FRACTION: f32 = 0.06;
/// Resume (Play) glyph height as a fraction of the display rect.
const SUSPENDED_GLYPH_HEIGHT_FRACTION: f32 = 0.16;
/// Marker color — light grey, readable over the scrimmed frame.
const SUSPENDED_OVERLAY_COLOR: egui::Color32 = egui::Color32::from_gray(230);
/// Marker font sizes snap to this step and clamp below, so resizing a
/// suspended window walks a few cached glyph sizes, not hundreds
/// (epaint's font atlas never evicts within a session).
const SUSPENDED_FONT_STEP: f32 = 8.0;
const SUSPENDED_FONT_MIN: f32 = 16.0;
const SUSPENDED_FONT_MAX: f32 = 120.0;

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

    pub(crate) fn reset_emulation_clock(&mut self) {
        self.last_update = None;
        self.field_debt = 0.0;
        self.audio_cushion_fields = 0;
        self.schedule.stop_service();
    }

    /// Entering the throttle runs one interval of fields ahead so the ring never
    /// drains between wake-ups; leaving it owes those fields back, so emulation
    /// idles while the cushion plays out and realigns with the wall clock.
    fn adjust_audio_cushion(&mut self, repaint_delay: Option<Duration>) {
        let desired = repaint_delay.map_or(0, |delay| {
            scheduling::cushion_fields(self.machine.config.video.field_rate_hz(), delay)
        });
        if desired > self.audio_cushion_fields {
            let before = self.fields_run;
            self.run_fields(desired - self.audio_cushion_fields);
            self.audio_cushion_fields += (self.fields_run - before) as usize;
        } else if desired < self.audio_cushion_fields {
            self.field_debt -= (self.audio_cushion_fields - desired) as f64;
            self.audio_cushion_fields = desired;
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
    pub(crate) fn run_fields(&mut self, n: usize) {
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
        self.fields_due_at(std::time::Instant::now())
    }

    fn fields_due_at(&mut self, now: std::time::Instant) -> usize {
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
    /// `repaint_delay` describes this viewport's background presentation policy.
    /// A fast-forward in progress (`app/fast_forward.rs`) replaces the
    /// wall-clock field loop with one unthrottled slice, drops that slice's
    /// audio, and asks for the next repaint right away.
    pub(crate) fn step_emulation(&mut self, ctx: &egui::Context, repaint_delay: Option<Duration>) {
        self.poll_drivewire_host();
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        let fast_forwarding = self.running && self.is_fast_forwarding();
        if self.running {
            if fast_forwarding {
                self.run_fast_forward_slice();
            } else {
                let cushion = repaint_delay.unwrap_or_else(|| {
                    scheduling::foreground_cushion(self.machine.config.video.field_rate_hz())
                });
                self.run_emulation_fields(Some(cushion));
            }
            // Only the save half runs here — re-finalizing would fold a same-field
            // capture into the already-landed recording, discarding its leader.
            if self.machine.bus.cassette.take_recording_landed()
                && let Err(e) = self.save_tape_bytes()
            {
                self.cart_error = Some(e);
            }
            self.emit_audio(fast_forwarding);
        }
        if !self.running {
            // Includes breakpoints hit above, even if no paused repaint follows.
            self.reset_emulation_clock();
        }
        self.poll_drivewire_host();

        self.upload_framebuffer_texture(ctx);
        if self.running {
            self.schedule_next_service(ctx, repaint_delay);
        }
    }

    /// Feed the fields' audio to the output, or discard it for a
    /// fast-forward slice: unthrottled fields produce audio far faster than
    /// the device plays it, and nothing of it is worth hearing at that
    /// speed. The ring runs dry meanwhile and the output fades to silence
    /// (`audio::UNDERRUN_FADE_SECS`), as it does over any gap.
    fn emit_audio(&mut self, discard: bool) {
        let sample_rate = self.machine.audio_sample_rate();
        // `take_audio` drains the machine's buffer either way; a discarded
        // `Drain` removes its samples without yielding them.
        let samples = self.machine.take_audio();
        if !discard {
            self.audio.push_samples(samples, sample_rate);
        }
    }

    /// Ask for the next frame: right away while a fast-forward is still in
    /// progress, so its slices run back to back; otherwise at the next
    /// service deadline of the wall-clock cadence.
    fn schedule_next_service(&mut self, ctx: &egui::Context, repaint_delay: Option<Duration>) {
        if self.is_fast_forwarding() {
            ctx.request_repaint();
            return;
        }
        let now = std::time::Instant::now();
        let interval =
            scheduling::service_interval(self.machine.config.video.field_rate_hz(), repaint_delay);
        let deadline = self.schedule.service_deadline(now, interval);
        scheduling::request_repaint_at(ctx, deadline);
    }

    /// Upload the framebuffer as `self.texture`. This is the only part of
    /// [`Self::step_emulation`] that a suspended VM's window still runs, so its
    /// picture stays on screen without input handling.
    pub(crate) fn upload_framebuffer_texture(&mut self, ctx: &egui::Context) {
        self.upload_framebuffer_texture_at(ctx, std::time::Instant::now());
    }

    /// [`Self::upload_framebuffer_texture`] at an explicit instant, so tests
    /// can place repaints within or across a presentation interval.
    pub(crate) fn upload_framebuffer_texture_at(
        &mut self,
        ctx: &egui::Context,
        now: std::time::Instant,
    ) {
        let background = scheduling::background_delay(ctx);
        let mut interval =
            scheduling::service_interval(self.machine.config.video.field_rate_hz(), background);
        let active = self.running && !self.suspended;
        let animated = matches!(self.display, Display::TV(_)) && self.tv.noise_pct > 0;
        if !active {
            interval = background.unwrap_or(super::presentation::NOISE_INTERVAL);
        }
        let due = self.schedule.presentation_due(now, interval);
        if self.texture.is_none() || (!active && !animated) || due {
            self.present_framebuffer(ctx, now);
        }
        // Running VMs animate on their service ticks. A separate 60 Hz noise
        // timer would interleave with the machine's slightly different field rate.
        if !active && animated {
            scheduling::request_repaint_at(ctx, self.schedule.presentation_deadline());
        }
    }

    fn present_framebuffer(&mut self, ctx: &egui::Context, now: std::time::Instant) {
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

    /// The CoCo display: the letterboxed 4:3 framebuffer
    /// texture. Requires [`Self::step_emulation`] to have already run this frame.
    pub(crate) fn draw_display(&mut self, ui: &mut egui::Ui) {
        let tex = self.texture.as_ref().unwrap();
        let aspect = TARGET_ASPECT;
        // Largest 4:3 rect that fits the panel, centered (letterboxed).
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
        if self.suspended && suspended_overlay(ui, rect) {
            self.pending_resume = true;
        }
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
    /// share popup memory with the manager window — and so is the
    /// machine-type prompt, dropping its state unloaded.
    fn suspended_window_ui(&mut self, ctx: &egui::Context) {
        if !self.drew_suspended {
            egui::Popup::close_all(ctx);
            self.pending_load = None;
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

/// Grey scrim with a centered Play glyph and a "Suspended" marker halfway
/// between it and the bottom edge (as `Label`s so AccessKit exposes them) on
/// the frozen display. The whole rect is clickable; returns true when clicked
/// to request a resume.
fn suspended_overlay(ui: &mut egui::Ui, rect: egui::Rect) -> bool {
    ui.painter().rect_filled(rect, 0.0, SUSPENDED_SCRIM);
    let glyph_font = overlay_font(rect.height() * SUSPENDED_GLYPH_HEIGHT_FRACTION);
    let text_font = overlay_font(rect.height() * SUSPENDED_TEXT_HEIGHT_FRACTION);
    let glyph_rect =
        egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), glyph_font.size));
    // The marker sits halfway between the Play glyph and the bottom edge.
    let text_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, (glyph_rect.bottom() + rect.bottom()) / 2.0),
        egui::vec2(rect.width(), text_font.size),
    );
    for (r, s, font) in [
        (glyph_rect, PLAY_GLYPH, glyph_font),
        (text_rect, SUSPENDED_OVERLAY_TEXT, text_font),
    ] {
        let text = egui::RichText::new(s)
            .font(font)
            .color(SUSPENDED_OVERLAY_COLOR);
        ui.put(r, egui::Label::new(text).selectable(false));
    }
    ui.interact(
        rect,
        ui.id().with("suspended_overlay"),
        egui::Sense::click(),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

/// Marker font at `px`, snapped to [`SUSPENDED_FONT_STEP`] and clamped.
fn overlay_font(px: f32) -> egui::FontId {
    let size = (px / SUSPENDED_FONT_STEP).round() * SUSPENDED_FONT_STEP;
    egui::FontId::proportional(size.clamp(SUSPENDED_FONT_MIN, SUSPENDED_FONT_MAX))
}

#[cfg(test)]
#[path = "frame_test.rs"]
mod tests;

#[cfg(test)]
#[path = "presentation_integration_test.rs"]
mod presentation_tests;
