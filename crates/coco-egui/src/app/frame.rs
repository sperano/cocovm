//! The per-frame loop: crediting wall-clock time to emulated fields,
//! running them, and getting the resulting framebuffer onto the screen.

use coco_core::video::SQUARE_PIXEL_ASPECT;

use crate::*;

impl CocoApp {
    /// Emulated fields owed for this update, from wall-clock time at the
    /// machine's field rate (60 Hz NTSC / 50 Hz PAL).
    pub(crate) fn fields_due(&mut self) -> usize {
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

    /// Advance emulation for one host frame — input, joysticks, the
    /// wall-clock-paced field loop, audio, and the framebuffer texture
    /// upload. Runs regardless of which chrome (if any) is drawn around the
    /// display this frame: [`Self::window_ui`] (full native window) and the
    /// manager's `ViewportClass::Embedded` fallback both call this before
    /// drawing anything, so a VM keeps emulating even in the degraded
    /// single-window case (`docs/plan-machine-persistence.md` "one native
    /// window per running VM").
    pub(crate) fn step_emulation(&mut self, ctx: &egui::Context) {
        self.handle_input(ctx);
        self.drive_joysticks(ctx);

        if self.running {
            // Run however many fields the wall clock owes us (real-time pacing),
            // stepping type-ahead per field so paste timing is refresh-agnostic.
            // Routed through the debugger so an enabled breakpoint/watchpoint
            // pauses the emulator cleanly instead of running straight through
            // it — a no-op when no breakpoints/watchpoints are set (the
            // common case), since `DebuggerPanel::run_field` then always
            // completes the field, same as `Machine::run_field` directly.
            for _ in 0..self.fields_due() {
                if self.type_ahead.is_active() {
                    self.type_ahead.advance(&mut self.machine.bus.keyboard);
                }
                if !self.debugger.run_field(&mut self.machine) {
                    self.running = false;
                    break;
                }
            }
            let sample_rate = self.machine.audio_sample_rate();
            self.audio
                .push_samples(self.machine.take_audio(), sample_rate);
            ctx.request_repaint();
        } else {
            self.last_update = None;
            // Drop any fields owed to the wall clock while paused (debugger
            // pause included), so resuming doesn't instantly "catch up" on
            // the paused interval — a clean pause, not just a frozen screen.
            self.field_debt = 0.0;
        }

        self.upload_framebuffer_texture(ctx);
    }

    /// Upload the framebuffer as `self.texture` — [`Self::step_emulation`]'s
    /// final step, and the ONLY part of it a *suspended* VM's window runs
    /// (`manager::vm_windows`): a frozen machine must keep its picture on
    /// screen without `handle_input`/`drive_joysticks`, which would leave
    /// the quick-load/quick-save shortcuts, type-ahead, and keyboard/
    /// joystick writes live on a machine whose on-disk frozen copy they'd
    /// silently diverge from.
    pub(crate) fn upload_framebuffer_texture(&mut self, ctx: &egui::Context) {
        // The TV chain (B&W collapse, bandwidth limit, scanlines), run at
        // the single point every consumer of `self.texture` — VM window,
        // manager preview, embedded fallback — inherits from. The machine's
        // own framebuffer stays untouched: the effect is a display
        // preference, not state. The chain owns the output shape (scanline
        // doubling), hence the frame's own dimensions here.
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
        // NEAREST for monitors, LINEAR for TVs (`texture_options`'s doc).
        // Passed on every `set`, so switching the display in the status
        // bar's display menu re-filters the very next frame.
        let options = crate::display::texture_options(self.display);
        let texture = self
            .texture
            .get_or_insert_with(|| ctx.load_texture("coco-fb", image.clone(), options));
        texture.set(image, options);
    }

    /// The CoCo display itself: the letterboxed, (optionally) aspect-
    /// corrected framebuffer texture, filling whatever `ui` it's given.
    /// Split out of [`Self::window_ui`]'s `CentralPanel` closure so the
    /// manager's `ViewportClass::Embedded` fallback can show just this —
    /// without the rest of [`Self::draw_chrome`] — inside a plain
    /// `egui::Window` instead of a full-window `CentralPanel`
    /// (`docs/plan-machine-persistence.md` "one native window per running
    /// VM"). Requires [`Self::step_emulation`] to have already run this
    /// frame (it uploads `self.texture`, `unwrap`ped below).
    pub(crate) fn draw_display(&mut self, ui: &mut egui::Ui) {
        let tex = self.texture.as_ref().unwrap();
        // Aspect the displayed frame should have: the 4:3 tube when
        // corrected, else the square-pixel view of the machine's visible
        // window (identical for both renderer geometries — `video.rs`'s
        // constant doc). NOT derived from the texture: the TV chain's
        // scanline doubling changes the texture's shape but not the
        // picture's, so a tex-derived aspect would disagree between a
        // monitor and a TV showing the same machine.
        let aspect = if self.aspect_correct {
            TARGET_ASPECT
        } else {
            SQUARE_PIXEL_ASPECT
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
    }

    /// The full app window for one frame: emulation step, every menu/toolbar/
    /// dialog, then the display, in that order — exactly the body
    /// `eframe::App::update` ran before this method existed. `pub(crate)` so
    /// the manager's per-VM immediate viewport (`manager.rs`'s
    /// `draw_running_vms`, `ViewportClass::Default`/native case) can call it
    /// directly on a VM it owns, drawing this same full chrome inside its own
    /// native OS window (`docs/plan-machine-persistence.md` "one native
    /// window per running VM"). The `eframe::App` impl below (test scaffolding
    /// only — see its doc comment in `app.rs`) just forwards here.
    pub(crate) fn window_ui(&mut self, ctx: &egui::Context) {
        self.step_emulation(ctx);
        self.draw_chrome(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| self.draw_display(ui));
    }
}
