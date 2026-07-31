//! Machine lifecycle: creating, starting, stopping, renaming, and deleting
//! entries — everything that changes *which* machines exist or whether
//! their VM is running, as opposed to editing one's definition in place
//! (`manager::detail`).

use std::fs;
use std::path::PathBuf;

use coco_core::MachineConfig;

use crate::machine_def;

use super::{
    MachineEntry, ManagerApp, NO_CONFIG_DIR, NO_DATA_DIR, SUSPEND_STATE_FILE, THUMBNAIL_FILE,
    suspend_state_path,
};

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
        let created = Some(
            chrono::Local::now()
                .format(machine_def::DATE_FORMAT)
                .to_string(),
        );
        let def = machine_def::MachineDef::from_config(name, created, &MachineConfig::default());
        match machine_def::save(&dir, &slug, &def) {
            Ok(()) => {
                let index = self.entries.partition_point(|e| e.slug < slug);
                self.entries.insert(index, MachineEntry::new(slug, def));
                self.selection.set_single(index);
                self.edit = None; // seeded from the new entry on next draw
                self.focus_name = true;
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Play on a Powered Off machine: launch `entries[index]`'s *saved*
    /// definition (`crate::launch_machine`) — not the in-progress edit
    /// draft, which may hold changes the user hasn't saved yet (the small
    /// note next to the button in [`super::detail`]'s `draw_detail_ok` is
    /// the only warning about that). A powered-off entry always has
    /// `vm: None`, so this only ever replaces `None` with `Some`; callers
    /// route a Running or Suspended entry elsewhere ([`Self::resume_vm`]).
    pub(super) fn start_vm(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.launch_error = None;
        match crate::launch_machine(&entry.def, &entry.slug) {
            Ok(vm) => entry.vm = Some(Box::new(vm)),
            Err(e) => entry.launch_error = Some(e),
        }
    }

    /// Suspend (the ⏸ transport button, Running machines only): freeze the
    /// machine to disk and pause it in place. Order matters — the
    /// screenshot first (the row preview must show the exact frozen frame),
    /// then the state file (`CocoApp::save_state_to`, which flushes dirty
    /// media itself as part of its contract), then the pause. A failed save
    /// aborts the whole suspend: the machine stays Running and the error
    /// lands in the transport row's error label (`launch_error` — same
    /// label Start uses). The VM window deliberately stays open; closing it
    /// is the user's choice ([`super::vm_windows`] just drops the VM object
    /// for a suspended entry, the state being safe on disk).
    pub(super) fn suspend_vm(&mut self, index: usize) {
        let Some(path) = self.suspend_state_path_for(index) else {
            self.entries[index].launch_error = Some(NO_DATA_DIR.to_string());
            return;
        };
        if self.entries[index].vm.is_none() {
            return;
        }
        self.write_entry_thumbnail(index);
        if let Some(dir) = path.parent()
            && let Err(e) = fs::create_dir_all(dir)
        {
            self.entries[index].launch_error = Some(format!("{}: {e}", dir.display()));
            return;
        }
        let entry = &mut self.entries[index];
        let vm = entry.vm.as_mut().expect("checked Some above");
        match vm.save_state_to(&path) {
            Ok(()) => {
                vm.set_running(false);
                entry.suspended = true;
                entry.launch_error = None;
            }
            Err(e) => entry.launch_error = Some(e),
        }
    }

    /// Play on a Suspended machine: bring it back to Running. Two shapes —
    /// the VM object may still be alive (suspend never closes the window),
    /// in which case resuming is just un-pausing; or the window was closed
    /// (VM dropped), in which case a fresh launch restores the frozen state
    /// over itself (`CocoApp::load_state_from` replaces the machine
    /// wholesale, so what the launch booted is irrelevant — it only has to
    /// succeed). Either way a successful resume deletes the
    /// [`super::SUSPEND_STATE_FILE`]: the running machine immediately
    /// diverges from the frozen copy, and a stale file would misreport
    /// Suspended after the next power-off. A failed relaunch/restore keeps
    /// the file and the Suspended state — the frozen copy is still the
    /// truth, and the error shows in the transport row.
    pub(super) fn resume_vm(&mut self, index: usize) {
        let Some(path) = self.suspend_state_path_for(index) else {
            self.entries[index].launch_error = Some(NO_DATA_DIR.to_string());
            return;
        };
        if self.entries[index].vm.is_none() {
            self.start_vm(index);
            let entry = &mut self.entries[index];
            let Some(vm) = entry.vm.as_mut() else {
                return; // launch failed; start_vm already recorded the error
            };
            if let Err(e) = vm.load_state_from(&path) {
                entry.vm = None;
                entry.launch_error = Some(e);
                return;
            }
        }
        let entry = &mut self.entries[index];
        entry
            .vm
            .as_mut()
            .expect("alive or just restored")
            .set_running(true);
        entry.suspended = false;
        entry.launch_error = None;
        if let Err(e) = fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            // The machine IS running; a leftover state file is only a
            // misleading label after the next power-off — worth a warning,
            // not worth failing the resume.
            tracing::warn!("could not remove {}: {e}", path.display());
        }
    }

    /// Stop — the power switch (⏹ button, row context menu, and a *running*
    /// VM window's close box via [`super::vm_windows`]'s
    /// `close_vm_window`): flush dirty disks/tape back to their files —
    /// the same exit contract `CocoApp::on_exit` runs for the direct-boot
    /// window — then drop the VM, returning the row to Powered Off. On a
    /// Suspended machine (VM alive or not) this also discards the frozen
    /// state file — powering off is explicitly "throw the saved state
    /// away". The saved screenshot is deleted along with it (not just the
    /// cached texture): a powered-off machine has no preview, and a stale
    /// PNG left behind would resurface via `write_thumbnail_png`'s
    /// keep-previous-on-black rule as a *previous power cycle's* screen the
    /// next time Suspend fires during a blanked display.
    pub(super) fn stop_vm(&mut self, index: usize) {
        if let Some(mut vm) = self.entries[index].vm.take() {
            vm.flush_media();
        }
        let entry = &mut self.entries[index];
        entry.suspended = false;
        entry.thumbnail = None;
        entry.thumbnail_load_attempted = false;
        if let Some(root) = &self.artifacts_root {
            let dir = root.join(&entry.slug);
            for file in [SUSPEND_STATE_FILE, THUMBNAIL_FILE] {
                if let Err(e) = fs::remove_file(dir.join(file))
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("could not remove {file} for '{}': {e}", entry.slug);
                }
            }
        }
    }

    /// `entries[index]`'s [`SUSPEND_STATE_FILE`] path — `None` when no
    /// artifact root exists (no home directory), which disables
    /// Suspend/Resume outright. Returns an owned path so callers keep their
    /// `&mut self` freedom.
    fn suspend_state_path_for(&self, index: usize) -> Option<PathBuf> {
        Some(suspend_state_path(
            self.artifacts_root.as_deref()?,
            &self.entries[index].slug,
        ))
    }

    /// Rename `entries[index]`'s `<slug>.toml` and artifact directory to
    /// match its (already saved) display name. The slug is the identity
    /// (`machine_def.rs` "Identity = slug") and nothing else persists it —
    /// relative `[media]` entries name files *inside* the artifact dir —
    /// so a rename is exactly these two filesystem moves, uniquified like
    /// create. Only safe with the machine Powered Off — a running VM writes
    /// `thumbnail.png` into the artifact dir by path, and a *suspended*
    /// machine's `suspended.ccstate` records the media's absolute
    /// pre-rename paths (`save_state_to`'s `MediaRefs`), so moving the
    /// directory under it would make the frozen state unrestorable; callers
    /// guard on both. The list is re-sorted afterwards, with the selection,
    /// the edit state, and every pending delete all following their entry.
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
        let selection_snapshot = self.selection.snapshot(&self.entries);
        let entry = self.entries.remove(index);
        let at = self.entries.partition_point(|e| e.slug < entry.slug);
        self.entries.insert(at, entry);
        self.selection.restore(&self.entries, &selection_snapshot);
        if let Some(edit) = self.edit.as_mut()
            && edit.slug == old
        {
            edit.slug = new.clone();
        }
        for slug in &mut self.pending_delete {
            if *slug == old {
                *slug = new.clone();
            }
        }
    }

    /// Run once per `update()`, before any panel draws (so row indices stay
    /// stable for the whole frame): migrate the slug of every renamed
    /// machine that is Powered Off — not running AND not suspended, since a
    /// suspended machine's frozen state pins the artifact dir's old path
    /// (see [`Self::migrate_slug`]'s doc); its rename stays pending until
    /// the next power-off. Several can be pending at once (rename a running
    /// machine, select another, rename it too…), and each
    /// [`Self::migrate_slug`] re-sorts the list — hence re-`position` from
    /// scratch per iteration rather than iterating indices. Terminates
    /// because `migrate_slug` clears `rename_pending` unconditionally,
    /// success or failure.
    pub(super) fn apply_pending_renames(&mut self) {
        while let Some(index) = self
            .entries
            .iter()
            .position(|e| e.rename_pending && e.vm.is_none() && !e.suspended)
        {
            self.migrate_slug(index);
        }
    }
}
