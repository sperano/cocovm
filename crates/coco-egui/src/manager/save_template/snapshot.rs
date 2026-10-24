//! The pure half of Save as Template: the [`snapshot`] of a machine's
//! definition that becomes the template, and the file list the dialog
//! shows ([`file_lines`]).

use std::path::Path;

use crate::machine_def::{MachineDef, StatsDTO};
use crate::manager::asset_downloads::megabytes;
use crate::manager::templates::media::{self, relative_media};

/// The relative path of the machine's data folder itself.
const DATA_FOLDER: &str = ".";

/// Below this, a size reads in kilobytes rather than megabytes.
const BYTES_PER_KB: u64 = 1000;
const BYTES_PER_MB: u64 = BYTES_PER_KB * BYTES_PER_KB;

/// `def` as a template named `name`, dated `created`, with zeroed
/// `[stats]`. Absolute paths into the machine's own `artifact_dir` become
/// relative, so they are copied like the relative ones and never point
/// back at the machine; the print folder follows too, without its
/// contents, even when it is the data folder itself. `[hardware].rom` is
/// left alone: a relative one would resolve in the working directory
/// instead.
pub(crate) fn snapshot(
    def: &MachineDef,
    artifact_dir: Option<&Path>,
    name: String,
    created: Option<String>,
) -> MachineDef {
    let mut snapshot = MachineDef {
        name,
        created,
        stats: StatsDTO::default(),
        ..def.clone()
    };
    let Some(dir) = artifact_dir else {
        return snapshot;
    };
    crate::path_remap::remap_definition_paths(&mut snapshot, dir, Path::new(""));
    snapshot.hardware.rom.clone_from(&def.hardware.rom);
    // A share of the data folder itself would come out empty: keep it.
    for (share, original) in snapshot
        .drivewire
        .shares
        .iter_mut()
        .zip(&def.drivewire.shares)
    {
        if share.path.is_empty() {
            share.path.clone_from(&original.path);
        }
    }
    let folder = &mut snapshot.drivewire.printer.folder;
    if let Ok(inside) = Path::new(folder.as_str()).strip_prefix(dir) {
        *folder = if inside.as_os_str().is_empty() {
            DATA_FOLDER.to_string()
        } else {
            inside.to_string_lossy().into_owned()
        };
    }
    snapshot
}

/// What saving does with one file the snapshot names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileFate {
    /// Copied into the template's media folder; this many bytes.
    Copied(u64),
    /// Absolute, outside the machine's data folder: the template refers to
    /// the same file.
    Shared,
    /// Named relative to the machine's data folder, but not there: saving
    /// is refused.
    Missing,
    /// Relative, but climbing out of the machine's data folder (`..`):
    /// saving is refused.
    Outside,
}

impl FileFate {
    /// Whether the file keeps the template from being saved.
    pub(crate) fn blocks_save(&self) -> bool {
        matches!(self, Self::Missing | Self::Outside)
    }
}

/// One line of the dialog's file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileLine {
    pub(crate) path: String,
    pub(crate) fate: FileFate,
}

impl FileLine {
    /// The line as the dialog shows it.
    pub(crate) fn text(&self) -> String {
        match self.fate {
            FileFate::Copied(bytes) => format!("{} ({}) is copied", self.path, size_text(bytes)),
            FileFate::Shared => format!("{} stays shared", self.path),
            FileFate::Missing => format!("{} is missing from the machine's folder", self.path),
            FileFate::Outside => format!("{} is outside the machine's folder", self.path),
        }
    }
}

/// `bytes` in kilobytes, or in megabytes from one megabyte up.
fn size_text(bytes: u64) -> String {
    if bytes < BYTES_PER_MB {
        format!("{} KB", bytes.div_ceil(BYTES_PER_KB))
    } else {
        megabytes(bytes)
    }
}

/// What saving `snapshot` does with each file it names, the relative ones
/// resolved in `artifact_dir`. A relative path that climbs out of it is
/// never inspected.
pub(crate) fn file_lines(snapshot: &MachineDef, artifact_dir: Option<&Path>) -> Vec<FileLine> {
    let relative = relative_media(snapshot);
    let resolve = |path: &str| artifact_dir.map(|dir| dir.join(path));
    let outside = |path: &str| media::check_inside(path).is_err();
    let files = relative.files.iter().map(|path| FileLine {
        path: path.clone(),
        fate: if outside(path) {
            FileFate::Outside
        } else {
            match resolve(path).and_then(|file| std::fs::metadata(file).ok()) {
                Some(meta) if meta.is_file() => FileFate::Copied(meta.len()),
                _ => FileFate::Missing,
            }
        },
    });
    let skip = artifact_dir.and_then(|dir| relative.print_dir(dir));
    let folders = relative
        .folders
        .iter()
        .filter_map(|path| {
            if outside(path) {
                return Some((path, FileFate::Outside));
            }
            let source = resolve(path).filter(|source| media::is_folder(source))?;
            let bytes = media::tree_bytes(&source, skip.as_deref());
            Some((path, FileFate::Copied(bytes)))
        })
        .map(|(path, fate)| FileLine {
            path: path.clone(),
            fate,
        });
    let shared = snapshot
        .host_paths()
        .into_iter()
        .filter(|path| Path::new(path).is_absolute())
        .map(|path| FileLine {
            path: path.to_string(),
            fate: FileFate::Shared,
        });
    files.chain(folders).chain(shared).collect()
}

#[cfg(test)]
#[path = "snapshot_test.rs"]
mod tests;
