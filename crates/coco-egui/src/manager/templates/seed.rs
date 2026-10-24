//! Writing the built-in templates ([`Preset`]) into the templates
//! directory, where they become ordinary template files.
//!
//! A record in that directory ([`SEEDED_RECORD`]) lists the
//! [`Preset::id`]s already written. Each launch writes only the ones it
//! lacks, so a built-in template a later release adds reaches existing
//! installs, and one the user deleted never comes back. A release that
//! changes a built-in template never touches the user's file.

use std::fs;
use std::io;
use std::path::Path;

use super::{Template, TemplateDirs, media};
use crate::machine_def;
use crate::manager::detail_map::{blank_disk_file, place_blank_file};
use crate::manager::presets::{self, BLANK_DISK_DRIVE, Preset};
use crate::nitros9::{self, EOU_IMAGES};

/// The file in the templates directory listing the [`Preset::id`]s already
/// seeded, one per line. A dotfile, so it never loads as a template.
const SEEDED_RECORD: &str = ".builtin-templates";

/// Write every built-in template that [`SEEDED_RECORD`] doesn't list yet,
/// dated `created`, and add it to the record. Holds the same kind of lock
/// as starter-machine seeding (`presets::lock_seeding`), so two instances
/// launched at once never write the same template twice.
///
/// The NitrOS-9 EOU template's images are moved in from `legacy_eou_dir`,
/// where earlier releases downloaded them, when they are there; otherwise
/// the New Machine dialog offers their download.
pub(crate) fn seed_builtins(
    dirs: &TemplateDirs,
    legacy_eou_dir: Option<&Path>,
    created: Option<String>,
) -> Result<(), String> {
    let (Some(defs), Some(media_root)) = (dirs.defs.as_deref(), dirs.media_root.as_deref()) else {
        return Ok(());
    };
    fs::create_dir_all(defs).map_err(|e| format!("{}: {e}", defs.display()))?;
    let _lock = presets::lock_seeding(defs)?;
    let record = defs.join(SEEDED_RECORD);
    let mut seeded = read_record(&record)?;
    for preset in Preset::ALL {
        if seeded.iter().any(|id| id == preset.id()) {
            continue;
        }
        let slug = seed_one(preset, dirs, defs, media_root, created.clone())?;
        seeded.push(preset.id().to_string());
        if let Err(error) = write_record(&record, &seeded) {
            // Unrecorded, it would be seeded again beside this copy.
            let _ = super::delete(dirs, &slug);
            return Err(error);
        }
        if preset == Preset::NitrOS9Eou
            && let Some(legacy) = legacy_eou_dir
        {
            move_eou_images(legacy, &media_root.join(slug));
        }
    }
    Ok(())
}

/// Write `preset` as a template: its media folder and blank disk first,
/// then its file, so a failure never leaves a template without its media.
/// Returns its slug.
fn seed_one(
    preset: Preset,
    dirs: &TemplateDirs,
    defs: &Path,
    media_root: &Path,
    created: Option<String>,
) -> Result<String, String> {
    let base = machine_def::slugify(preset.name());
    let slug = media::claim(media_root, &base, &|candidate| dirs.slug_taken(candidate))?;
    let template = Template {
        slug: slug.clone(),
        def: preset.def(created),
        description: preset.description().to_string(),
    };
    let written =
        place_blank_disk(preset, media_root, &slug).and_then(|()| super::save(defs, &template));
    if written.is_err() {
        media::discard(&media_root.join(&slug));
    }
    written.map(|()| slug)
}

/// The blank floppy `preset` mounts, if any, in media folder `slug`.
fn place_blank_disk(preset: Preset, media_root: &Path, slug: &str) -> Result<(), String> {
    if !preset.has_blank_disk() {
        return Ok(());
    }
    place_blank_file(media_root, slug, &blank_disk_file(BLANK_DISK_DRIVE))
}

/// The ids in `record`; none when it doesn't exist yet.
fn read_record(record: &Path) -> Result<Vec<String>, String> {
    match fs::read_to_string(record) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", record.display())),
    }
}

/// Write `ids` to `record`, through a temporary sibling renamed into place.
fn write_record(record: &Path, ids: &[String]) -> Result<(), String> {
    let tmp = record.with_extension("tmp");
    let mut text = ids.join("\n");
    text.push('\n');
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, record).map_err(|e| format!("{}: {e}", record.display()))
}

/// Move the [`EOU_IMAGES`] an earlier release downloaded into `legacy`
/// into the EOU template's `media_dir`, then remove `legacy` once nothing
/// else is in it. Best effort: an image left behind only means the New
/// Machine dialog offers the download again.
fn move_eou_images(legacy: &Path, media_dir: &Path) {
    for name in EOU_IMAGES {
        let source = legacy.join(name);
        if !source.is_file() {
            continue;
        }
        let target = media_dir.join(name);
        // Across file systems a rename fails; copy, then remove the source.
        let moved = fs::rename(&source, &target).or_else(|_| {
            let copied = fs::copy(&source, &target).and_then(|_| fs::remove_file(&source));
            if copied.is_err() {
                // Never leave a partial image under its real name.
                let _ = fs::remove_file(&target);
            }
            copied
        });
        if let Err(e) = moved {
            eprintln!(
                "coco: moving {} into {}: {e}",
                source.display(),
                media_dir.display()
            );
        }
    }
    let _ = fs::remove_file(legacy.join(nitros9::LOCK_FILE));
    let _ = fs::remove_dir(legacy);
}

#[cfg(test)]
#[path = "seed_test.rs"]
mod tests;
