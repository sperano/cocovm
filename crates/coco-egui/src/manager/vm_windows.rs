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
/// wins pointer routing at a given position). A small preview loses nothing
/// real: this fallback only ever shows the bare display, never chrome.
const EMBEDDED_FALLBACK_SIZE: egui::Vec2 = egui::vec2(320.0, 240.0);

/// Window size of a launched VM's own native OS window: the same formula
/// `main()` uses for the direct-boot window (`main.rs`'s `SCALE`/
/// `TARGET_ASPECT`/`MENU_BAR_H`/`TOOLBAR_H`/`STATUS_BAR_H`), sized for the
/// aspect-corrected (wider) image so it always fits.
fn vm_window_inner_size() -> egui::Vec2 {
    let img_h = coco_core::video::FB_H as f32 * crate::SCALE;
    let win_w = img_h * crate::TARGET_ASPECT;
    let win_h = img_h + crate::MENU_BAR_H + crate::TOOLBAR_H + crate::STATUS_BAR_H;
    egui::vec2(win_w, win_h)
}

impl ManagerApp {
    /// One native OS window per running VM (`docs/plan-machine-persistence.md`
    /// "DECIDED: in-process, one native window per running VM"): an
    /// immediate viewport per entry with a VM, keyed by a stable id derived
    /// from the slug so egui reuses the same OS window across frames instead
    /// of respawning it (the same pattern `paper_view::PaperWindow::ui` uses
    /// for the printer-paper window). Called once per `ManagerApp::update`,
    /// after the manager's own panels.
    ///
    /// Close requests (the native window's close box, or the embedded
    /// fallback's `egui::Window` close button) are collected into a list and
    /// applied after the loop — for a Running machine the close box IS the
    /// power switch ([`Self::stop_vm`]); for a Suspended one it merely
    /// drops the VM object, the frozen state staying on disk. Deferred
    /// because `stop_vm` needs `&mut self.entries[i]`, which would conflict
    /// with the `vm` this loop already holds taken out of that same slot
    /// for the duration of the viewport closure.
    pub(super) fn draw_running_vms(&mut self, ctx: &egui::Context) {
        let mut to_stop: Vec<usize> = Vec::new();
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

            // Taken out of the entry so the viewport closure below can hold
            // and mutate it without a conflicting borrow of `self` (the
            // closure also needs to push into `to_stop`, a local, not
            // `self` — so no `self` borrow is held across the closure at
            // all here).
            let mut vm = self.entries[i].vm.take().expect("checked Some above");
            let mut close_requested = false;
            ctx.show_viewport_immediate(viewport_id, builder, |child_ctx, class| {
                if class == egui::ViewportClass::Embedded {
                    // Degraded single-window fallback (kittest and other
                    // backends without native multi-window support, per
                    // `paper_view`'s module doc comment on the same
                    // pattern): don't draw `CocoApp`'s own menu bar/toolbar/
                    // status bar into the manager's shared `ctx` — that
                    // would interleave two independent sets of panels into
                    // one window. Show just the VM's display in a plain
                    // `egui::Window` instead; full chrome only exists as its
                    // own native OS window. The VM still runs:
                    // `step_emulation` is unconditional either way.
                    vm.step_emulation(child_ctx);
                    let mut open = true;
                    // Anchored, and capped at `EMBEDDED_FALLBACK_SIZE`
                    // rather than the native window's full
                    // `inner_size` (found the hard way, via a kittest
                    // regression: a window that large, even anchored to a
                    // corner, still spans most of a modest single-window
                    // canvas — e.g. the whole manager UI under kittest — and
                    // silently eats clicks meant for the manager's own
                    // panels underneath, since pointer routing goes to
                    // whichever window is topmost at that screen position.
                    // This fallback only ever shows the bare display anyway
                    // (no chrome), so a smaller preview loses nothing a
                    // real native window wouldn't already provide instead.
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
                    vm.window_ui(child_ctx);
                    if child_ctx.input(|i| i.viewport().close_requested()) {
                        close_requested = true;
                    }
                }
            });

            self.entries[i].vm = Some(vm);
            if close_requested {
                to_stop.push(i);
            }
        }
        for i in to_stop {
            if self.entries[i].suspended {
                // Closing a *suspended* machine's window is not the power
                // switch — the frozen state is already safe on disk
                // (`save_state_to` flushed dirty media as part of Suspend,
                // and the paused machine can't have dirtied anything
                // since), so just drop the VM object; the row keeps showing
                // its suspend-time screenshot.
                self.entries[i].vm = None;
            } else {
                self.stop_vm(i);
            }
        }
    }
}
