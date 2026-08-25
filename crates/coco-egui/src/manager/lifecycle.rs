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
/// ([`ManagerApp::create_machine_now`]) — [`MachineConfig::default`]'s model.
fn default_new_name() -> String {
    crate::machine_label(MachineConfig::default().variant).to_string()
}

impl ManagerApp {
    /// "New…" (toolbar button and ⌘N): create a default machine right now —
    /// saved to disk, inserted in the list, and selected with the Name field
    /// focused, instead of opening a dialog. Does NOT boot anything.
    pub(super) fn create_machine_now(&mut self) {
        let Some(dir) = self.machines_dir.clone() else {
            self.save_error = Some(NO_CONFIG_DIR.to_string());
            return;
        };
        let name = default_new_name();
        // Checks both the in-memory list and the directory itself, since
        // `entries` misses a file written by another instance.
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

    /// Play on a Powered Off machine: launch `entries[index]`'s saved
    /// definition, not the in-progress edit draft. Counts as a fresh start
    /// ([`Self::record_start`]).
    pub(super) fn start_vm(&mut self, index: usize) {
        if self.launch_vm(index) {
            self.record_start(index);
        }
    }

    /// Launch `entries[index]`'s saved definition into a fresh `CocoApp`,
    /// with no stats bookkeeping. Returns whether the launch succeeded.
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
    /// by [`Self::start_vm`] after a successful fresh launch only.
    fn record_start(&mut self, index: usize) {
        self.entries[index].def.stats.starts += 1;
        self.save_entry_def(index);
    }

    /// Persist `entries[index]`'s current definition to its `<slug>.toml`,
    /// clearing or recording [`ManagerApp::save_error`]. On success, also
    /// resyncs a live edit session's auto-save baseline for this entry, if
    /// one is open, so the next keystroke's repack doesn't redundantly re-save.
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

    /// Write `entries[index]`'s live VM's runtime into its persisted
    /// `[stats].runtime_secs`, when it actually changed. A no-op for an
    /// entry with no live VM.
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

    /// Suspend (Running machines only): freeze the machine to disk and
    /// pause it in place. A failed save aborts the whole suspend — the
    /// machine stays Running and the error surfaces in `launch_error`.
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

    /// Play on a Suspended machine: bring it back to Running, either by
    /// un-pausing the still-alive VM or, if the window was closed,
    /// relaunching and restoring the frozen state. Either way that relaunch
    /// never counts as a fresh start, and the [`super::SUSPEND_STATE_FILE`]
    /// is deleted only once Running is confirmed — a resume that can't
    /// delete it fails and the entry stays Suspended.
    pub(super) fn resume_vm(&mut self, index: usize) {
        let Some(path) = self.suspend_state_path_for(index) else {
            self.entries[index].launch_error = Some(NO_DATA_DIR.to_string());
            return;
        };
        let cold = self.entries[index].vm.is_none();
        if cold {
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
        // NotFound still counts as consumed — the VM already holds the
        // state, so there's nothing left to misreport Suspended.
        if let Err(e) = fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            let entry = &mut self.entries[index];
            if cold {
                entry.vm = None; // back to Suspended with the window closed
            }
            entry.launch_error = Some(format!("could not remove {}: {e}", path.display()));
            return;
        }
        let entry = &mut self.entries[index];
        entry
            .vm
            .as_mut()
            .expect("alive or just restored")
            .set_running(true);
        entry.suspended = false;
        entry.launch_error = None;
    }

    /// Stop — the power switch: fold runtime, flush dirty media, then drop
    /// the VM regardless of the flush outcome. On a Suspended machine this
    /// also discards the frozen state file and its preview; a failed
    /// discard leaves the entry Suspended. Errors surface in `launch_error`.
    pub(super) fn stop_vm(&mut self, index: usize) {
        self.fold_runtime_into_def(index);
        let flush_error = self.entries[index]
            .vm
            .take()
            .and_then(|mut vm| vm.flush_media().err());
        let entry = &mut self.entries[index];
        let mut discard_error = None;
        if let Some(root) = &self.artifacts_root {
            let dir = root.join(&entry.slug);
            let state_path = dir.join(SUSPEND_STATE_FILE);
            match fs::remove_file(&state_path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    discard_error =
                        Some(format!("could not discard {}: {e}", state_path.display()));
                }
                _ => {
                    entry.suspended = false;
                    entry.thumbnail = None;
                    entry.thumbnail_load_attempted = false;
                    if let Err(e) = fs::remove_file(dir.join(THUMBNAIL_FILE))
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        tracing::warn!(
                            "could not remove {THUMBNAIL_FILE} for '{}': {e}",
                            entry.slug
                        );
                    }
                }
            }
        } else {
            // No artifact root means Suspend never wrote a state file or
            // thumbnail, so there's nothing on disk to reconcile.
            entry.suspended = false;
            entry.thumbnail = None;
            entry.thumbnail_load_attempted = false;
        }
        entry.launch_error = match (flush_error, discard_error) {
            (Some(flush), Some(discard)) => Some(format!("{flush}\n{discard}")),
            (either, None) | (None, either) => either,
        };
    }

    /// `entries[index]`'s [`SUSPEND_STATE_FILE`] path — `None` when no
    /// artifact root exists, which disables Suspend/Resume outright.
    fn suspend_state_path_for(&self, index: usize) -> Option<PathBuf> {
        Some(suspend_state_path(
            self.artifacts_root.as_deref()?,
            &self.entries[index].slug,
        ))
    }

    /// Rename `entries[index]`'s `<slug>.toml` and artifact directory to
    /// match its saved display name. Only safe with the machine Powered
    /// Off — a running or suspended machine's paths must not move out from
    /// under it; callers guard on both.
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
                // Roll back: a stale slug beats [media] entries resolving
                // against a mismatched directory.
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

    /// Run once per `update()`, before any panel draws: migrate the slug of
    /// every renamed machine that is Powered Off. Re-finds indices from
    /// scratch each iteration since [`Self::migrate_slug`] re-sorts the list.
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
