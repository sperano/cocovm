//! Native OS windows for running VMs: [`ManagerApp::draw_running_vms`], one
//! immediate viewport per entry with a VM.

use eframe::egui;

use super::ManagerApp;

/// Default size of the `ViewportClass::Embedded` fallback's `egui::Window`
/// (`draw_running_vms`) — deliberately much smaller than
/// [`vm_window_inner_size`]'s full native-window formula. That size (which
/// includes room for a menu bar/toolbar/status bar this fallback never
/// draws) is often close to or larger than the *entire* embedded canvas, so
/// even anchored to a corner it can span most of the screen and silently
/// eat clicks meant for the manager's own panels underneath (topmost window
/// wins pointer routing at a given position). A small preview loses no
/// functionality: this fallback only ever shows the bare display, never chrome.
const EMBEDDED_FALLBACK_SIZE: egui::Vec2 = egui::vec2(320.0, 240.0);

/// Window size of a launched VM's own native OS window, sized for the
/// aspect-corrected (wider) image so it always fits.
fn vm_window_inner_size() -> egui::Vec2 {
    let img_h = coco_core::video::FB_H as f32 * crate::SCALE;
    let win_w = img_h * crate::TARGET_ASPECT;
    let win_h = img_h + crate::MENU_BAR_H + crate::TOOLBAR_H + crate::STATUS_BAR_H;
    egui::vec2(win_w, win_h)
}

impl ManagerApp {
    /// One native OS window per running VM: an immediate viewport per
    /// entry, keyed by a stable id from the slug so egui reuses the same OS
    /// window across frames. Called once per `ManagerApp::update`, after
    /// the manager's own panels.
    pub(super) fn draw_running_vms(&mut self, ctx: &egui::Context) {
        // Indices suspended or resumed this frame, for `focus_first_failed_row`
        // later to focus the first failure.
        let mut acted: Vec<usize> = Vec::new();
        // One app-wide decision: every VM viewport repaints the manager anyway.
        let repaint_delay = ctx.input(|i| {
            crate::app::background_repaint_delay(i.raw.viewports.values().map(|v| v.focused))
        });
        for i in 0..self.entries.len() {
            if self.entries[i].vm.is_none() {
                continue;
            }
            let slug = self.entries[i].slug.clone();
            let name = self.entries[i].def.name.clone();
            let viewport_id = egui::ViewportId::from_hash_of(("vm-window", &slug));
            let inner_size = vm_window_inner_size();
            let builder = egui::ViewportBuilder::default()
                .with_title(name.clone())
                .with_inner_size(inner_size);

            // Taken out of the entry so the closure can mutate it without conflicting with `self`.
            let mut vm = self.entries[i].vm.take().expect("checked Some above");
            let suspended = self.entries[i].suspended;
            vm.suspended = suspended;
            let mut close_requested = false;
            ctx.show_viewport_immediate(viewport_id, builder, |child_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Embedded fallback: draws only the VM's display in a
                    // plain `egui::Window`, never the full chrome, to avoid
                    // interleaving two panel sets into one window.
                    if suspended {
                        vm.upload_framebuffer_texture(child_ctx);
                    } else {
                        vm.step_emulation(child_ctx, repaint_delay);
                    }
                    let mut open = true;
                    // Capped at EMBEDDED_FALLBACK_SIZE, not the native
                    // window's full size, which would eat clicks meant for
                    // the manager's panels.
                    egui::Window::new(crate::window_title(child_ctx, &name))
                        .id(egui::Id::new(("vm-window-embedded", slug.as_str())))
                        .open(&mut open)
                        .resizable(false)
                        .default_size(EMBEDDED_FALLBACK_SIZE)
                        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -8.0))
                        .show(child_ctx, |ui| {
                            vm.draw_display(ui);
                        });
                    if !open {
                        close_requested = true;
                    }
                } else {
                    // A suspended window (`vm.suspended`, set above) shows the
                    // frozen frame under read-only chrome: nothing in it may
                    // diverge the machine from the on-disk state.
                    vm.window_ui(child_ctx, repaint_delay);
                    if child_ctx.input(|i| i.viewport().close_requested()) {
                        close_requested = true;
                    }
                }
            });

            // Reads the requests here rather than threading them out of the
            // branch that can set them, so these `take`s are correct
            // regardless of which branch ran.
            let suspend_requested = std::mem::take(&mut vm.pending_suspend);
            let resume_requested = std::mem::take(&mut vm.pending_resume);
            self.entries[i].vm = Some(vm);
            if close_requested {
                // Close wins over suspend — closing already tears the VM
                // down, discarding whatever suspend would have frozen
                // anyway.
                self.close_vm_window(i);
            } else if suspend_requested && !suspended {
                self.suspend_vm(i);
                acted.push(i);
            } else if resume_requested && suspended {
                self.resume_vm(i);
                acted.push(i);
            }
        }
        // Focuses the first failed row across the batch; on success the
        // window flips to display-only on its own next frame.
        self.focus_first_failed_row(&acted);
    }

    /// The VM window's close box: the power switch for a Running machine
    /// ([`Self::stop_vm`]), but for a Suspended one merely drops the VM
    /// object — the frozen state is already on disk from suspend time.
    pub(crate) fn close_vm_window(&mut self, index: usize) {
        if self.entries[index].suspended {
            self.entries[index].vm = None;
        } else {
            self.stop_vm(index);
        }
    }
}
