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
/// model, the same default `ui_tests::harness::boot_harness` boots for tests.
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
    /// Counts as a fresh start ([`Self::record_start`]) — [`Self::resume_vm`]
    /// launches through [`Self::launch_vm`] directly for its cold-relaunch
    /// case, so *that* launch never counts.
    pub(super) fn start_vm(&mut self, index: usize) {
        if self.launch_vm(index) {
            self.record_start(index);
        }
    }

    /// Launch `entries[index]`'s saved definition into a fresh `CocoApp`,
    /// with no stats bookkeeping — the mechanics [`Self::start_vm`] (a fresh
    /// start, which counts) and [`Self::resume_vm`]'s cold-relaunch case (a
    /// launch immediately overwritten by a restored snapshot, which must
    /// not) both need underneath. Returns whether the launch succeeded.
    fn launch_vm(&mut self, index: usize) -> bool {
        self.entries[index].launch_error = None;
        match crate::launch_machine(&self.entries[index].def, &self.entries[index].slug) {
            Ok(vm) => {
                self.entries[index].vm = Some(Box::new(vm));
                true
            }
            Err(e) => {
                self.entries[index].launch_error = Some(e);
                false
            }
        }
    }

    /// Increment `starts` and persist `entries[index]`'s definition — called
    /// by [`Self::start_vm`] after a successful fresh launch only. Resume's
    /// cold-relaunch path calls [`Self::launch_vm`] directly and never
    /// reaches this, so neither a Suspend → close window → Resume cycle nor
    /// a Suspend → quit → relaunch → Resume one (`entry.suspended`
    /// rehydrates from the on-disk `.ccstate` either way) counts as a start.
    fn record_start(&mut self, index: usize) {
        self.entries[index].def.stats.starts += 1;
        self.save_entry_def(index);
    }

    /// Persist `entries[index]`'s current definition to its `<slug>.toml`,
    /// clearing [`ManagerApp::save_error`] on success or recording the
    /// failure otherwise (the convention `manager::detail`'s `autosave` and
    /// `commit_name` both follow) — the write path a stats-only change (no
    /// form edit involved) uses, bypassing the detail pane's `autosave`
    /// dirty-check. On success, also keeps a live edit session for this same
    /// entry in step: if `self.edit` is seeded from `entries[index]`, its
    /// auto-save baseline (`EditState::packed`) is resynced to the
    /// just-saved definition, the same way `commit_name` keeps `packed.name`
    /// current — otherwise the next keystroke's repack would see a stats-only
    /// diff and redundantly re-save.
    fn save_entry_def(&mut self, index: usize) {
        let Some(dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        match machine_def::save(&dir, &self.entries[index].slug, &self.entries[index].def) {
            Ok(()) => {
                self.save_error = None;
                if let Some(edit) = self.edit.as_mut()
                    && edit.slug == self.entries[index].slug
                {
                    edit.packed = self.entries[index].def.clone();
                }
            }
            Err(e) => self.save_error = Some(e),
        }
    }

    /// Write `entries[index]`'s live VM's [`CocoApp::total_runtime`] into its
    /// persisted `[stats].runtime_secs`, when it actually changed. Idempotent:
    /// a fold within the same second as the last one (nothing new accrued at
    /// whole-second granularity) writes nothing, since `total_runtime` only
    /// ever grows and truncating it to whole seconds loses no information —
    /// the fractional remainder simply stays in `total_runtime` for the next
    /// fold to pick up. A no-op for an entry with no live VM. Shared by
    /// [`Self::suspend_vm`], [`Self::stop_vm`], and [`ManagerApp::on_exit`]
    /// (`manager.rs`) — each calls this while the VM is still in place
    /// (`entries[index].vm`), before whatever happens to it next (pause,
    /// `take()`, flush).
    ///
    /// The def's runtime total only ever advances through this method while
    /// a VM is alive (launch seeds `total_runtime` from the def — see
    /// `launch::launch_machine` — and the field only grows from there), so
    /// this assignment can't go backwards in practice.
    pub(super) fn fold_runtime_into_def(&mut self, index: usize) {
        let Some(vm) = self.entries[index].vm.as_ref() else {
            return;
        };
        let total = vm.total_runtime.as_secs();
        if total != self.entries[index].def.stats.runtime_secs {
            self.entries[index].def.stats.runtime_secs = total;
            self.save_entry_def(index);
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
        if let Err(e) = vm.save_state_to(&path) {
            entry.launch_error = Some(e);
            return;
        }
        vm.set_running(false);
        entry.suspended = true;
        entry.launch_error = None;
        self.fold_runtime_into_def(index);
    }

    /// Play on a Suspended machine: bring it back to Running. Two shapes —
    /// the VM object may still be alive (suspend never closes the window),
    /// in which case resuming is just un-pausing; or the window was closed
    /// (VM dropped), in which case a fresh launch restores the frozen state
    /// over itself (`CocoApp::load_state_from` replaces the machine
    /// wholesale, so what the launch booted is irrelevant — it only has to
    /// succeed). That cold-relaunch case goes through [`Self::launch_vm`]
    /// directly, never [`Self::start_vm`]: Resume must never count as a
    /// fresh start (`machine_def::StatsDTO::starts`'s doc), whether the VM
    /// object survived or had to be relaunched — and whether the app itself
    /// stayed up (window closed) or was quit and relaunched (`entry.suspended`
    /// rehydrates from the on-disk `.ccstate` either way). Either way a
    /// successful resume deletes the
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
            self.launch_vm(index);
            let entry = &mut self.entries[index];
            let Some(vm) = entry.vm.as_mut() else {
                return; // launch failed; launch_vm already recorded the error
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
    /// `close_vm_window`): fold the session's runtime
    /// ([`Self::fold_runtime_into_def`], while the VM is still in
    /// `entries[index].vm` for it to find), flush dirty disks/tape back to
    /// their files (`CocoApp::flush_media`, the same one
    /// `eframe::App::on_exit` calls for the test-only window in `app.rs`) —
    /// then drop the VM, returning the row to Powered Off. On a
    /// Suspended machine (VM alive or not) this also discards the frozen
    /// state file — powering off is explicitly "throw the saved state
    /// away". The saved screenshot is deleted along with it (not just the
    /// cached texture): a powered-off machine has no preview, and a stale
    /// PNG left behind would resurface via `write_thumbnail_png`'s
    /// keep-previous-on-black rule as a *previous power cycle's* screen the
    /// next time Suspend fires during a blanked display.
    pub(super) fn stop_vm(&mut self, index: usize) {
        self.fold_runtime_into_def(index);
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

#[cfg(test)]
#[path = "lifecycle_test.rs"]
mod tests;
