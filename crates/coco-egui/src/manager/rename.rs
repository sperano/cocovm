//! Recoverable machine rename transactions.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{machine_def, path_remap};

use super::{ManagerApp, SUSPEND_STATE_FILE, detail_map};

const RENAME_JOURNAL_FILE: &str = ".rename-journal.toml";
const RENAME_JOURNAL_TMP_FILE: &str = ".rename-journal.toml.tmp";
const RENAME_STAGED_SLUG: &str = ".rename-target";
const RENAME_BACKUP_FILE: &str = ".rename-source.toml";

#[derive(Deserialize, Serialize)]
struct RenameJournal {
    old_slug: String,
    new_slug: String,
}

pub(super) struct PendingRename {
    pub(super) slug: String,
    pub(super) name: String,
}

struct RenamePlan {
    journal: RenameJournal,
    machines_dir: PathBuf,
    old_config: PathBuf,
    new_config: PathBuf,
    staged_config: PathBuf,
    backup_config: PathBuf,
    old_artifacts: Option<PathBuf>,
    new_artifacts: Option<PathBuf>,
    /// Present for a live transaction. Recovery uses the already-durable
    /// staged configuration instead.
    target_def: Option<machine_def::MachineDef>,
}

fn config_path(dir: &Path, slug: &str) -> PathBuf {
    dir.join(format!("{slug}.toml"))
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, bytes).map_err(|error| format!("{}: {error}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).map_err(|error| format!("{}: {error}", path.display()))
}

fn write_journal(plan: &RenamePlan) -> Result<(), String> {
    let text = toml::to_string(&plan.journal)
        .map_err(|error| format!("serializing rename journal: {error}"))?;
    let tmp_path = plan.machines_dir.join(RENAME_JOURNAL_TMP_FILE);
    let final_path = plan.machines_dir.join(RENAME_JOURNAL_FILE);
    fs::write(&tmp_path, text).map_err(|error| format!("{}: {error}", tmp_path.display()))?;
    fs::rename(&tmp_path, &final_path).map_err(|error| format!("{}: {error}", final_path.display()))
}

fn prepare(plan: &RenamePlan) -> Result<(), String> {
    let target_def = plan
        .target_def
        .as_ref()
        .expect("a live rename plan carries its target definition");
    machine_def::save(&plan.machines_dir, RENAME_STAGED_SLUG, target_def)?;
    if let Err(error) = write_journal(plan) {
        let _ = remove_if_present(&plan.staged_config);
        return Err(error);
    }
    Ok(())
}

fn move_artifacts(plan: &RenamePlan) -> Result<bool, String> {
    let (Some(old_dir), Some(new_dir)) = (&plan.old_artifacts, &plan.new_artifacts) else {
        return Ok(false);
    };
    if !old_dir.exists() {
        return Ok(false);
    }
    fs::rename(old_dir, new_dir).map_err(|error| format!("{}: {error}", old_dir.display()))?;
    Ok(true)
}

fn rewrite_checkpoint(plan: &RenamePlan) -> Result<Option<Vec<u8>>, String> {
    let (Some(old_dir), Some(new_dir)) = (&plan.old_artifacts, &plan.new_artifacts) else {
        return Ok(None);
    };
    let state_path = new_dir.join(SUSPEND_STATE_FILE);
    if !state_path.is_file() {
        return Ok(None);
    }
    let original = fs::read(&state_path)
        .map_err(|error| format!("could not read {}: {error}", state_path.display()))?;
    let mut payload = coco_core::snapshot::load(&original).map_err(|error| error.to_string())?;
    path_remap::remap_media_refs(&mut payload.media, old_dir, new_dir);
    let bytes = coco_core::snapshot::save(&payload.machine, &payload.media)
        .map_err(|error| error.to_string())?;
    write_atomic(&state_path, &bytes)?;
    Ok(Some(original))
}

fn install_config(plan: &RenamePlan) -> Result<(), String> {
    fs::rename(&plan.old_config, &plan.backup_config)
        .map_err(|error| format!("{}: {error}", plan.old_config.display()))?;
    if let Err(error) = fs::rename(&plan.staged_config, &plan.new_config) {
        let _ = fs::rename(&plan.backup_config, &plan.old_config);
        return Err(format!("{}: {error}", plan.new_config.display()));
    }
    Ok(())
}

fn rollback_artifacts(
    plan: &RenamePlan,
    artifacts_moved: bool,
    original_checkpoint: Option<&[u8]>,
) -> Result<(), String> {
    if !artifacts_moved {
        return Ok(());
    }
    let old_dir = plan
        .old_artifacts
        .as_deref()
        .expect("moved artifact source");
    let new_dir = plan
        .new_artifacts
        .as_deref()
        .expect("moved artifact target");
    if let Some(bytes) = original_checkpoint {
        write_atomic(&new_dir.join(SUSPEND_STATE_FILE), bytes)?;
    }
    fs::rename(new_dir, old_dir).map_err(|error| format!("{}: {error}", new_dir.display()))
}

fn discard_preparation(plan: &RenamePlan) -> Result<(), String> {
    remove_if_present(&plan.staged_config)?;
    remove_if_present(&plan.machines_dir.join(RENAME_JOURNAL_FILE))
}

fn rollback(
    plan: &RenamePlan,
    artifacts_moved: bool,
    original_checkpoint: Option<&[u8]>,
) -> Result<(), String> {
    rollback_artifacts(plan, artifacts_moved, original_checkpoint)?;
    discard_preparation(plan)
}

fn perform(plan: &RenamePlan) -> Result<(), String> {
    prepare(plan)?;
    let artifacts_moved = match move_artifacts(plan) {
        Ok(moved) => moved,
        Err(error) => {
            let _ = discard_preparation(plan);
            return Err(error);
        }
    };
    let original_checkpoint = match rewrite_checkpoint(plan) {
        Ok(original) => original,
        Err(error) => {
            let rollback = rollback(plan, artifacts_moved, None).err();
            return Err(join_errors(error, rollback));
        }
    };
    if let Err(error) = install_config(plan) {
        let rollback = rollback(plan, artifacts_moved, original_checkpoint.as_deref()).err();
        return Err(join_errors(error, rollback));
    }
    if remove_if_present(&plan.backup_config).is_ok() {
        let _ = remove_if_present(&plan.machines_dir.join(RENAME_JOURNAL_FILE));
    }
    Ok(())
}

fn join_errors(error: String, rollback: Option<String>) -> String {
    match rollback {
        Some(rollback) => format!("{error}\nrename rollback failed: {rollback}"),
        None => error,
    }
}

impl ManagerApp {
    pub(super) fn queue_rename(&mut self, slug: String, name: String) {
        self.pending_rename = Some(PendingRename { slug, name });
    }

    pub(super) fn resolved_rename_slug(&self, old_slug: &str, name: &str) -> String {
        let base = machine_def::slugify(name);
        let taken = |candidate: &str| {
            if candidate == old_slug {
                return false;
            }
            self.entries.iter().any(|entry| entry.slug == candidate)
                || self
                    .machines_dir
                    .as_ref()
                    .is_some_and(|dir| config_path(dir, candidate).exists())
                || self
                    .artifacts_root
                    .as_ref()
                    .is_some_and(|root| root.join(candidate).exists())
        };
        machine_def::unique_slug(&base, &taken)
    }

    pub(super) fn save_name_only(&mut self, index: usize, name: String) -> Result<(), String> {
        let machines_dir = self
            .machines_dir
            .as_deref()
            .ok_or_else(|| super::NO_CONFIG_DIR.to_string())?;
        let mut new_def = self.entries[index].def.clone();
        new_def.name = name;
        machine_def::save(machines_dir, &self.entries[index].slug, &new_def)?;
        self.entries[index].def = new_def;
        Ok(())
    }

    fn rename_plan(&self, index: usize, name: String) -> Result<RenamePlan, String> {
        let machines_dir = self
            .machines_dir
            .clone()
            .ok_or_else(|| super::NO_CONFIG_DIR.to_string())?;
        let old_slug = self.entries[index].slug.clone();
        let new_slug = self.resolved_rename_slug(&old_slug, &name);
        let mut target_def = self.entries[index].def.clone();
        target_def.name = name;
        let (old_artifacts, new_artifacts) = match &self.artifacts_root {
            Some(root) => (Some(root.join(&old_slug)), Some(root.join(&new_slug))),
            None => (None, None),
        };
        if let (Some(old_dir), Some(new_dir)) = (&old_artifacts, &new_artifacts) {
            path_remap::remap_definition_paths(&mut target_def, old_dir, new_dir);
        }
        Ok(RenamePlan {
            journal: RenameJournal {
                old_slug: old_slug.clone(),
                new_slug: new_slug.clone(),
            },
            old_config: config_path(&machines_dir, &old_slug),
            new_config: config_path(&machines_dir, &new_slug),
            staged_config: config_path(&machines_dir, RENAME_STAGED_SLUG),
            backup_config: machines_dir.join(RENAME_BACKUP_FILE),
            machines_dir,
            old_artifacts,
            new_artifacts,
            target_def: Some(target_def),
        })
    }

    fn commit_rename(&mut self, index: usize, plan: RenamePlan) {
        let old_slug = plan.journal.old_slug.clone();
        let new_slug = plan.journal.new_slug.clone();
        if let (Some(old_dir), Some(new_dir), Some(vm)) = (
            plan.old_artifacts.as_deref(),
            plan.new_artifacts.as_deref(),
            self.entries[index].vm.as_mut(),
        ) {
            vm.remap_managed_paths(old_dir, new_dir);
        }
        self.entries[index].def = plan
            .target_def
            .expect("a completed live rename carries its target definition");
        self.entries[index].slug = new_slug.clone();
        self.rekey_rename(index, &old_slug, &new_slug);
    }

    fn rekey_rename(&mut self, index: usize, old_slug: &str, new_slug: &str) {
        debug_assert_eq!(self.entries[index].slug, new_slug);
        self.apply_manager_sort(self.manager_sort);
        let destination = self
            .entries
            .iter()
            .position(|entry| entry.slug == new_slug)
            .expect("renamed entry remains in the list");
        let mut refreshed_form = detail_map::seed_form(&self.entries[destination].def);
        let refreshed_packed = self
            .pack_def(
                &self.entries[destination].def,
                new_slug,
                &mut refreshed_form,
            )
            .expect("a seeded renamed form always packs");
        if let Some(edit) = self.edit.as_mut()
            && edit.slug == old_slug
        {
            edit.slug = new_slug.to_string();
            edit.name = self.entries[destination].def.name.clone();
            edit.form = refreshed_form;
            edit.packed = refreshed_packed;
        }
        for slug in &mut self.pending_delete {
            if slug == old_slug {
                *slug = new_slug.to_string();
            }
        }
        for pending in &mut self.pending {
            pending.retarget(old_slug, new_slug);
        }
    }

    pub(super) fn apply_pending_rename(&mut self) {
        let Some(pending) = self.pending_rename.take() else {
            return;
        };
        if let Some(machines_dir) = self.machines_dir.as_deref()
            && let Err(error) = recover_pending_rename(machines_dir, self.artifacts_root.as_deref())
        {
            self.save_error = Some(error);
            return;
        }
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.slug == pending.slug)
        else {
            return;
        };
        if self.resolved_rename_slug(&pending.slug, &pending.name) == pending.slug {
            match self.save_name_only(index, pending.name) {
                Ok(()) => {
                    self.apply_manager_sort(self.manager_sort);
                    let index = self
                        .entries
                        .iter()
                        .position(|entry| entry.slug == pending.slug)
                        .expect("renamed entry remains in the list");
                    if let Some(edit) = self.edit.as_mut()
                        && edit.slug == pending.slug
                    {
                        edit.name = self.entries[index].def.name.clone();
                        edit.packed.name = self.entries[index].def.name.clone();
                    }
                    self.save_error = None;
                }
                Err(error) => self.save_error = Some(error),
            }
            return;
        }
        let plan = match self.rename_plan(index, pending.name) {
            Ok(plan) => plan,
            Err(error) => {
                self.save_error = Some(error);
                return;
            }
        };
        match perform(&plan) {
            Ok(()) => {
                self.commit_rename(index, plan);
                self.save_error = None;
            }
            Err(error) => self.save_error = Some(error),
        }
    }
}

fn read_journal(machines_dir: &Path) -> Result<Option<RenameJournal>, String> {
    let path = machines_dir.join(RENAME_JOURNAL_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

pub(super) fn recover_pending_rename(
    machines_dir: &Path,
    artifacts_root: Option<&Path>,
) -> Result<(), String> {
    let Some(journal) = read_journal(machines_dir)? else {
        return remove_if_present(&config_path(machines_dir, RENAME_STAGED_SLUG));
    };
    let old_config = config_path(machines_dir, &journal.old_slug);
    let new_config = config_path(machines_dir, &journal.new_slug);
    let staged_config = config_path(machines_dir, RENAME_STAGED_SLUG);
    let backup_config = machines_dir.join(RENAME_BACKUP_FILE);
    let old_artifacts = artifacts_root.map(|root| root.join(&journal.old_slug));
    let new_artifacts = artifacts_root.map(|root| root.join(&journal.new_slug));
    let plan = RenamePlan {
        journal,
        machines_dir: machines_dir.to_path_buf(),
        old_config,
        new_config,
        staged_config,
        backup_config,
        old_artifacts,
        new_artifacts,
        target_def: None,
    };
    finish_recovery(&plan)
}

fn finish_recovery(plan: &RenamePlan) -> Result<(), String> {
    if let (Some(old_dir), Some(new_dir)) = (&plan.old_artifacts, &plan.new_artifacts) {
        match (old_dir.exists(), new_dir.exists()) {
            (true, false) => fs::rename(old_dir, new_dir)
                .map_err(|error| format!("{}: {error}", old_dir.display()))?,
            (true, true) => {
                return Err(format!(
                    "rename recovery found both {} and {}",
                    old_dir.display(),
                    new_dir.display()
                ));
            }
            (false, _) => {}
        }
        let _ = rewrite_checkpoint(plan)?;
    }
    if !plan.new_config.exists() {
        if plan.old_config.exists() && !plan.backup_config.exists() {
            fs::rename(&plan.old_config, &plan.backup_config)
                .map_err(|error| format!("{}: {error}", plan.old_config.display()))?;
        }
        fs::rename(&plan.staged_config, &plan.new_config)
            .map_err(|error| format!("{}: {error}", plan.new_config.display()))?;
    }
    remove_if_present(&plan.old_config)?;
    remove_if_present(&plan.backup_config)?;
    remove_if_present(&plan.staged_config)?;
    remove_if_present(&plan.machines_dir.join(RENAME_JOURNAL_FILE))
}

#[cfg(test)]
#[path = "rename_test.rs"]
mod tests;
