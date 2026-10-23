//! The pure half of Save as Template: the [`snapshot`] of a machine's
//! definition that becomes the template, and the file list the dialog
//! shows ([`file_lines`]).

use std::path::Path;

use crate::machine_def::{MachineDef, StatsDTO};
use crate::manager::asset_downloads::megabytes;
use crate::manager::templates::media::{self, relative_media};

/// Below this, a size reads in kilobytes rather than megabytes.
const BYTES_PER_KB: u64 = 1000;
const BYTES_PER_MB: u64 = BYTES_PER_KB * BYTES_PER_KB;

/// `def` as a template named `name`, dated `created`, with zeroed
/// `[stats]`. Absolute paths into the machine's own `artifact_dir` become
/// relative, so they are copied like the relative ones and never point
/// back at the machine; the print folder follows too, without its
/// contents. `[hardware].rom` is left alone: a relative one would resolve
/// in the working directory instead.
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
    if let Ok(inside) = Path::new(folder.as_str()).strip_prefix(dir)
        && !inside.as_os_str().is_empty()
    {
        *folder = inside.to_string_lossy().into_owned();
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
/// resolved in `artifact_dir`.
pub(crate) fn file_lines(snapshot: &MachineDef, artifact_dir: Option<&Path>) -> Vec<FileLine> {
    let relative = relative_media(snapshot);
    let resolve = |path: &str| artifact_dir.map(|dir| dir.join(path));
    let files = relative.files.iter().map(|path| FileLine {
        path: path.clone(),
        fate: match resolve(path).and_then(|file| std::fs::metadata(file).ok()) {
            Some(meta) if meta.is_file() => FileFate::Copied(meta.len()),
            _ => FileFate::Missing,
        },
    });
    let folders = relative
        .folders
        .iter()
        .filter_map(|path| {
            let source = resolve(path)?;
            source.is_dir().then_some((path, source))
        })
        .map(|(path, source)| FileLine {
            path: path.clone(),
            fate: FileFate::Copied(media::tree_bytes(&source)),
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
