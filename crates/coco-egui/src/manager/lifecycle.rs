//! Machine lifecycle: creating, starting, stopping, renaming, and deleting
//! entries — everything that changes *which* machines exist or whether
//! their VM is running, as opposed to editing one's definition in place
//! (`manager::detail`).

use std::fs;

use coco_core::MachineConfig;
use eframe::egui;

use crate::machine_def;

use super::{MachineEntry, ManagerApp, DETAIL_SECTION_GAP, NO_CONFIG_DIR};

/// Display name (and slug source) of a freshly created machine
/// ([`ManagerApp::create_machine_now`]) — [`MachineConfig::default`]'s
/// model, the same default the bare-invocation direct-boot path (`main.rs`)
/// starts from.
fn default_new_name() -> String {
    crate::machine_label(MachineConfig::default().variant).to_string()
}

impl ManagerApp {
    /// "New…" (toolbar button and ⌘N): create a default machine *right
    /// now* — saved to disk under a uniquified slug, inserted in the list,
    /// and selected with the Name field focused — instead of opening a
    /// dialog. There is no Cancel; an unwanted machine is deleted like any
    /// other (context menu → Delete…). Does NOT boot anything.
    pub(super) fn create_machine_now(&mut self) {
        let Some(dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let name = default_new_name();
        // Check both the in-memory list (loaded once at startup) and the
        // directory itself: `entries` misses any `<slug>.toml` written by a
        // second running instance or hand-placed since startup (an explicit
        // design goal — `machine_def.rs` module doc). Without the on-disk
        // check, `machine_def::save`'s unconditional rename would silently
        // overwrite that file.
        let taken = |candidate: &str| {
            self.entries.iter().any(|e| e.slug == candidate)
                || dir.join(format!("{candidate}.toml")).exists()
        };
        let slug = machine_def::unique_slug(&machine_def::slugify(&name), &taken);
        let created = Some(chrono::Local::now().format(machine_def::DATE_FORMAT).to_string());
        let def = machine_def::MachineDef::from_config(name, created, &MachineConfig::default());
        match machine_def::save(&dir, &slug, &def) {
            Ok(()) => {
                let index = self.entries.partition_point(|e| e.slug < slug);
                self.entries.insert(index, MachineEntry::new(slug, def));
                self.selected = Some(index);
                self.edit = None; // seeded from the new entry on next draw
                self.focus_name = true;
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Detail pane's Start button: launch `entries[index]`'s *saved*
    /// definition (`crate::launch_machine`) — not the in-progress edit
    /// draft, which may hold changes the user hasn't saved yet (the small
    /// note next to the button in [`super::detail`]'s `draw_detail_ok` is
    /// the only warning about that). A stopped entry always has `vm: None`,
    /// so this only ever replaces `None` with `Some`; an entry that's
    /// already running has no Start button to click (see the `is_running`
    /// match in `draw_detail_ok`).
    pub(super) fn start_vm(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.launch_error = None;
        match crate::launch_machine(&entry.def, &entry.slug) {
            Ok(vm) => entry.vm = Some(Box::new(vm)),
            Err(e) => entry.launch_error = Some(e),
        }
    }

    /// Stop button (and the VM window's own close box, via
    /// [`super::vm_windows`]'s `draw_running_vms`): flush dirty disks/tape
    /// back to their files — the same exit contract `CocoApp::on_exit` runs
    /// for the direct-boot window — then drop the VM, returning the row to
    /// Stopped.
    pub(super) fn stop_vm(&mut self, index: usize) {
        self.write_entry_thumbnail(index);
        if let Some(mut vm) = self.entries[index].vm.take() {
            vm.flush_media();
        }
    }

    /// Rename `entries[index]`'s `<slug>.toml` and artifact directory to
    /// match its (already saved) display name. The slug is the identity
    /// (`machine_def.rs` "Identity = slug") and nothing else persists it —
    /// relative `[media]` entries name files *inside* the artifact dir —
    /// so a rename is exactly these two filesystem moves, uniquified like
    /// create. Only safe with the VM stopped (a running VM writes
    /// `thumbnail.png` into the artifact dir by path); callers guard on
    /// that. The list is re-sorted afterwards, with `selected`, the edit
    /// state, and a pending delete all following their entry.
    fn migrate_slug(&mut self, index: usize) {
        self.entries[index].rename_pending = false;
        let Some(dir) = self.machines_dir.clone() else {
            return;
        };
        let old = self.entries[index].slug.clone();
        let base = machine_def::slugify(&self.entries[index].def.name);
        let slugs: Vec<String> = self.entries.iter().map(|e| e.slug.clone()).collect();
        let taken = |candidate: &str| {
            candidate != old
                && (slugs.iter().any(|s| s == candidate)
                    || dir.join(format!("{candidate}.toml")).exists())
        };
        let new = machine_def::unique_slug(&base, &taken);
        if new == old {
            return;
        }
        let old_path = dir.join(format!("{old}.toml"));
        let new_path = dir.join(format!("{new}.toml"));
        if let Err(e) = fs::rename(&old_path, &new_path) {
            self.save_error = Some(format!("{}: {e}", old_path.display()));
            return;
        }
        if let Some(root) = &self.artifacts_root {
            let old_dir = root.join(&old);
            if old_dir.exists()
                && let Err(e) = fs::rename(&old_dir, root.join(&new))
            {
                // Roll the definition back under the old slug: a stale slug
                // beats relative [media] entries resolving into a directory
                // that no longer matches the definition's file name.
                let _ = fs::rename(&new_path, &old_path);
                self.save_error = Some(format!("{}: {e}", old_dir.display()));
                return;
            }
        }
        self.entries[index].slug = new.clone();
        // Keep the list alphabetical and every slug-keyed pointer valid.
        let selected_slug = self.selected.map(|s| self.entries[s].slug.clone());
        let entry = self.entries.remove(index);
        let at = self.entries.partition_point(|e| e.slug < entry.slug);
        self.entries.insert(at, entry);
        if let Some(slug) = selected_slug {
            self.selected = self.entries.iter().position(|e| e.slug == slug);
        }
        if let Some(edit) = self.edit.as_mut()
            && edit.slug == old
        {
            edit.slug = new.clone();
        }
        if self.pending_delete.as_deref() == Some(old.as_str()) {
            self.pending_delete = Some(new);
        }
    }

    /// Run once per `update()`, before any panel draws (so row indices stay
    /// stable for the whole frame): migrate the slug of every renamed
    /// machine whose VM is gone. Several can be pending at once (rename a
    /// running machine, select another, rename it too…), and each
    /// [`Self::migrate_slug`] re-sorts the list — hence re-`position` from
    /// scratch per iteration rather than iterating indices. Terminates
    /// because `migrate_slug` clears `rename_pending` unconditionally,
    /// success or failure.
    pub(super) fn apply_pending_renames(&mut self) {
        while let Some(index) = self
            .entries
            .iter()
            .position(|e| e.rename_pending && e.vm.is_none())
        {
            self.migrate_slug(index);
        }
    }

    /// The confirmation modal behind the context menu's "Delete…"
    /// ([`ManagerApp::pending_delete`]), drawn once per `update()`. Esc,
    /// Cancel, and a click outside all dismiss without deleting; the confirm
    /// button reads "Stop and Delete" when the machine is running, since
    /// deleting stops it first. A failed delete reports its error inside the
    /// modal and leaves it open.
    pub(super) fn draw_delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(slug) = self.pending_delete.clone() else {
            return;
        };
        let Some(index) = self.entries.iter().position(|e| e.slug == slug) else {
            // The row vanished under the pending confirmation (see
            // `pending_delete`'s doc) — nothing left to delete.
            self.pending_delete = None;
            return;
        };
        let running = self.entries[index].vm.is_some();
        let name = self.entries[index].def.name.clone();
        let mut dismissed = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete_machine")).show(ctx, |ui| {
            ui.heading(format!("Delete “{name}”?"));
            ui.add_space(DETAIL_SECTION_GAP);
            ui.label(
                "The machine's definition is removed. Its disk, tape, and other \
                 media files stay on disk.",
            );
            if running {
                ui.label(
                    egui::RichText::new(
                        "This machine is running — it will be shut down first, like \
                         flipping the power switch; unsaved work inside it is lost.",
                    )
                    .strong(),
                );
            }
            if let Some(err) = &self.delete_error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            ui.add_space(DETAIL_SECTION_GAP);
            ui.horizontal(|ui| {
                let confirm = if running { "Stop and Delete" } else { "Delete" };
                if ui.button(confirm).clicked() {
                    self.delete_machine(index);
                }
                if ui.button("Cancel").clicked() {
                    dismissed = true;
                }
            });
        });
        if dismissed || modal.should_close() {
            self.pending_delete = None;
            self.delete_error = None;
        }
    }

    /// Confirmed delete of `entries[index]`: stop its VM if one is running
    /// (same flush contract as the Stop button), remove its `<slug>.toml`,
    /// and drop the row. Media/artifact files are deliberately left on disk
    /// (the modal says so). Failure lands in [`Self::delete_error`] with the
    /// entry kept, so the still-open modal can retry or cancel.
    fn delete_machine(&mut self, index: usize) {
        let Some(dir) = self.machines_dir.clone() else {
            self.delete_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let path = dir.join(format!("{}.toml", self.entries[index].slug));
        // A file already gone (deleted externally since startup) is fine —
        // the goal state "no definition on disk" is reached either way.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                self.delete_error = Some(format!("{}: {e}", path.display()));
                return;
            }
        }
        self.stop_vm(index);
        self.entries.remove(index);
        match self.selected {
            Some(s) if s == index => {
                self.selected = None;
                self.edit = None;
            }
            Some(s) if s > index => self.selected = Some(s - 1),
            _ => {}
        }
        self.pending_delete = None;
        self.delete_error = None;
    }
}
