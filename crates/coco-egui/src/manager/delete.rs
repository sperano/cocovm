//! The delete confirmation flow behind the list's "Delete…" (single- or
//! multi-row): the modal itself ([`ManagerApp::draw_delete_confirmation`]) and
//! the confirmed deletion it
//! commits ([`ManagerApp::commit_pending_delete`]) — split out of
//! `lifecycle.rs` to keep that module to the single-machine start/stop/
//! rename lifecycle.

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use eframe::egui;

use crate::{CocoApp, machine_def};

use super::{DETAIL_SECTION_GAP, MachineEntry, ManagerApp, NO_CONFIG_DIR};

impl ManagerApp {
    /// The confirmation modal behind "Delete…", drawn once per `update()`
    /// while [`ManagerApp::pending_delete`] holds at least one slug. Esc,
    /// Cancel, and a click outside all dismiss without deleting.
    pub(super) fn draw_delete_confirmation(&mut self, ctx: &egui::Context) {
        if self.pending_delete.is_empty() {
            return;
        }
        let indices: Vec<usize> = self
            .pending_delete
            .iter()
            .filter_map(|slug| self.entries.iter().position(|e| &e.slug == slug))
            .collect();
        if indices.is_empty() {
            self.pending_delete.clear();
            return;
        }
        let names: Vec<String> = indices
            .iter()
            .map(|&i| self.entries[i].def.name.clone())
            .collect();
        let any_running = self.bulk_flags(&indices).any_running;
        let any_suspended = indices.iter().any(|&i| self.entries[i].suspended);
        let mut dismissed = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete_machine")).show(ctx, |ui| {
            dismissed = self.draw_delete_confirmation_body(ui, &names, any_running, any_suspended);
        });
        if dismissed || modal.should_close() {
            self.pending_delete.clear();
            self.delete_error = None;
        }
    }

    /// The modal's contents: heading, [`draw_delete_warnings`], any error
    /// from a previous attempt, and the confirm/cancel row. Returns whether
    /// Cancel was clicked.
    fn draw_delete_confirmation_body(
        &mut self,
        ui: &mut egui::Ui,
        names: &[String],
        running: bool,
        suspended: bool,
    ) -> bool {
        if let [name] = names {
            ui.heading(format!("Delete “{name}”?"));
        } else {
            ui.heading(format!("Delete {} machines?", names.len()));
        }
        if names.len() > 1 {
            for name in names {
                ui.label(format!("• {name}"));
            }
        }
        ui.add_space(DETAIL_SECTION_GAP);
        ui.label(
            "The machine's definition and its data folder are removed, \
             including the disks, tapes, and suspend state stored there. \
             Media files elsewhere stay on disk.",
        );
        draw_delete_warnings(ui, names.len() > 1, running, suspended);
        if let Some(err) = &self.delete_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.add_space(DETAIL_SECTION_GAP);
        let mut dismissed = false;
        ui.horizontal(|ui| {
            let confirm = if running { "Stop and Delete" } else { "Delete" };
            if ui.button(confirm).clicked() {
                self.commit_pending_delete();
            }
            if ui.button("Cancel").clicked() {
                dismissed = true;
            }
        });
        dismissed
    }

    /// Confirmed delete: works through [`ManagerApp::pending_delete`] in
    /// order, resolving each slug to its current index right before
    /// deleting it, since each removal shifts indices after it. On the
    /// first failure, the failed slug and everything after it go back into
    /// `pending_delete` so the still-open modal can retry.
    ///
    /// `pending_delete` keeps the whole batch until the loop ends, so
    /// [`ManagerApp::check_data_folder_unshared`] can tell the batch's
    /// machines from the ones that stay.
    fn commit_pending_delete(&mut self) {
        let mut slugs = self.pending_delete.clone();
        for i in 0..slugs.len() {
            let Some(index) = self.entries.iter().position(|e| e.slug == slugs[i]) else {
                continue; // already gone
            };
            if let Err(e) = self.delete_machine(index) {
                self.delete_error = Some(e);
                self.pending_delete = slugs.split_off(i);
                return;
            }
        }
        self.pending_delete.clear();
        self.delete_error = None;
    }

    /// Delete `entries[index]`: stop its VM if running, remove its artifact
    /// dir and then its `<slug>.toml`, drop the row, and fix up
    /// selection/edit state. Media outside the artifact dir stays on disk.
    /// The definition goes last, so a failed removal leaves the machine
    /// listed for the modal's retry instead of orphaning its artifact dir.
    /// An artifact dir another machine uses refuses the delete before
    /// anything happens ([`ManagerApp::check_data_folder_unshared`]).
    fn delete_machine(&mut self, index: usize) -> Result<(), String> {
        let Some(dir) = self.machines_dir.clone() else {
            return Err(NO_CONFIG_DIR.to_string());
        };
        let slug = self.entries[index].slug.clone();
        let path = machine_def::def_path(&dir, &slug);
        let artifact_dir = self
            .artifacts_root
            .as_deref()
            .map(|root| artifact_dir(root, &slug))
            .transpose()?;
        if let Some(artifact_dir) = &artifact_dir {
            self.check_data_folder_unshared(index, artifact_dir)?;
        }
        // Stopping flushes dirty disk images, which may live in the artifact
        // dir, so it must finish before that dir goes.
        self.stop_vm(index);
        if let Some(artifact_dir) = &artifact_dir {
            remove_dir_if_present(artifact_dir)?;
        }
        // A file already gone is fine — the goal state is reached either way.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        self.entries.remove(index);
        self.selection.remove_index(index);
        if self.edit.as_ref().is_some_and(|e| e.slug == slug) {
            self.edit = None;
        }
        Ok(())
    }

    /// Refuse to delete `entries[index]` while a machine outside the pending
    /// batch names a file or folder inside its `artifact_dir` (by absolute
    /// path, or a relative one climbing out with `..`): removing the dir
    /// would take that file from the other machine. A running machine counts
    /// what it has mounted now as well as what its definition names. Paths
    /// are compared lexically, without following symlinks.
    fn check_data_folder_unshared(&self, index: usize, artifact_dir: &Path) -> Result<(), String> {
        let Some(root) = self.artifacts_root.as_deref() else {
            return Ok(());
        };
        let deleted = &self.entries[index];
        let dir = lexically_normalized(artifact_dir);
        let users: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.slug != deleted.slug && !self.pending_delete.contains(&e.slug))
            .filter(|e| uses_folder(e, root, &dir))
            .map(|e| format!("“{}”", e.def.name))
            .collect();
        if users.is_empty() {
            return Ok(());
        }
        let whom = if users.len() == 1 {
            "that machine"
        } else {
            "those machines"
        };
        Err(format!(
            "Can't delete “{}”: its data folder ({}) holds files used by {}. Remove them from \
             {whom} first, or delete {whom} too.",
            deleted.def.name,
            artifact_dir.display(),
            users.join(", "),
        ))
    }
}

/// Whether `entry` names a path inside `dir` (lexically normalized) in its
/// definition or, while running, in the media its VM has mounted: a runtime
/// Insert can mount a file the definition doesn't name.
fn uses_folder(entry: &MachineEntry, root: &Path, dir: &Path) -> bool {
    // Resolved like `machine_def::resolve_media_path`, against the injected
    // root.
    let own_dir = root.join(&entry.slug);
    let inside = |path: &Path| lexically_normalized(&own_dir.join(path)).starts_with(dir);
    entry
        .def
        .host_paths()
        .into_iter()
        .any(|raw| inside(Path::new(raw)))
        || entry
            .vm
            .as_deref()
            .is_some_and(|vm| live_paths(vm).any(inside))
}

/// The files `vm` has mounted or writes now: disks, hard disks, DriveWire
/// disks, tape, cartridge, and print capture.
fn live_paths(vm: &CocoApp) -> impl Iterator<Item = &Path> {
    vm.disk_paths
        .iter()
        .chain(&vm.vhd_paths)
        .chain(&vm.dw_paths)
        .chain([&vm.tape_path, &vm.cart_path, &vm.print_capture_path])
        .flatten()
        .map(PathBuf::as_path)
}

/// `path` with `.` dropped and each `..` cancelling the component before it,
/// without touching the file system (the files may not exist). Meant for
/// absolute paths: a leading `..` in a relative one is dropped.
fn lexically_normalized(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    normalized
}

/// `slug`'s artifact dir under `root`. A slug that isn't a single plain
/// file name (`load_all` never yields one) is refused rather than joined,
/// so a delete can never reach `root` itself or beyond it.
fn artifact_dir(root: &Path, slug: &str) -> Result<PathBuf, String> {
    if Path::new(slug).file_name() != Some(OsStr::new(slug)) {
        return Err(format!("refusing to delete the data folder of {slug:?}"));
    }
    Ok(root.join(slug))
}

/// Remove `dir` with everything in it. A dir already gone is fine, like the
/// definition file.
fn remove_dir_if_present(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// The two state warnings a delete confirmation shows — running machines are
/// shut down first, suspended ones lose their frozen state — worded plural
/// when more than one machine is involved.
fn draw_delete_warnings(ui: &mut egui::Ui, plural: bool, running: bool, suspended: bool) {
    if running {
        let text = if plural {
            "One or more of these machines are running — they will be shut down first, \
             like flipping the power switch; unsaved work inside them is lost."
        } else {
            "This machine is running — it will be shut down first, like flipping the \
             power switch; unsaved work inside it is lost."
        };
        ui.label(egui::RichText::new(text).strong());
    }
    if suspended {
        let text = if plural {
            "One or more of these machines are suspended — deleting discards their \
             frozen state."
        } else {
            "This machine is suspended — deleting discards its frozen state."
        };
        ui.label(egui::RichText::new(text).strong());
    }
}

#[cfg(test)]
#[path = "delete_test.rs"]
mod tests;
