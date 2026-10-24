//! The files that travel with a definition when it is copied: into a
//! template when a machine is saved as one, and out of it into each machine
//! made from it. These are the paths the definition names relative to its
//! data folder (a machine's artifact dir, or a template's media folder),
//! as [`relative_media`] lists them. Absolute paths name shared host files
//! and stay references.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::machine_def::{MachineDef, PrintOutputDTO};

/// A copy at most this large runs on the UI thread; a larger one (an EOU
/// system disk is 128 MB) on a thread of its own, behind a spinner. The
/// 0-byte blank disks of the built-in templates copy at once.
pub(crate) const BACKGROUND_COPY_BYTES: u64 = 16 * 1024 * 1024;

/// What a definition names relative to its data folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RelativeMedia {
    /// Disks, hard disks, the tape, DriveWire disks, and cartridge ROM
    /// images: files the machine needs.
    pub(crate) files: Vec<String>,
    /// DriveWire share folders, copied with what they hold. A missing one
    /// is skipped: launch creates it.
    pub(crate) folders: Vec<String>,
}

impl RelativeMedia {
    /// Whether the definition names nothing in its data folder.
    pub(crate) fn is_empty(&self) -> bool {
        self.files.is_empty() && self.folders.is_empty()
    }
}

/// The paths `def` names relative to its data folder, without
/// duplicates. The DriveWire print folder is left out: it holds the print
/// jobs of the machine that printed them, not media. A relative
/// `[hardware].rom` names a file in the working directory, so
/// `MachineDef::host_paths` already leaves it out.
pub(crate) fn relative_media(def: &MachineDef) -> RelativeMedia {
    let relative = |path: &&str| !Path::new(path).is_absolute();
    let folders: BTreeSet<&str> = def
        .drivewire
        .shares
        .iter()
        .map(|share| share.path.as_str())
        .filter(|path| !path.trim().is_empty())
        .filter(relative)
        .collect();
    let printer = &def.drivewire.printer;
    let print_folder =
        (printer.output == PrintOutputDTO::Folder).then_some(printer.folder.as_str());
    let files: BTreeSet<&str> = def
        .host_paths()
        .into_iter()
        .filter(relative)
        .filter(|path| !folders.contains(path) && Some(*path) != print_folder)
        .collect();
    RelativeMedia {
        files: files.into_iter().map(str::to_string).collect(),
        folders: folders.into_iter().map(str::to_string).collect(),
    }
}

/// The [`RelativeMedia::files`] of `def` missing from `media_dir`. A
/// template with any is listed but can't be used.
pub(crate) fn missing_files(def: &MachineDef, media_dir: &Path) -> Vec<String> {
    relative_media(def)
        .files
        .into_iter()
        .filter(|file| !media_dir.join(file).is_file())
        .collect()
}

/// Refuse a relative path that could leave its data folder (`..`), so a
/// copy never reads or writes outside the two folders it joins.
fn check_inside(path: &str) -> Result<(), String> {
    let inside = Path::new(path)
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if inside {
        Ok(())
    } else {
        Err(format!("{path} is outside the machine's data folder"))
    }
}

/// A copy of a definition's [`RelativeMedia`] from one data folder to
/// another.
#[derive(Debug, Clone)]
pub(crate) struct MediaCopy {
    from: PathBuf,
    /// The destination, which the caller claimed ([`claim`]) and removes
    /// on failure ([`discard`]).
    pub(crate) to: PathBuf,
    media: RelativeMedia,
}

impl MediaCopy {
    /// The copy of `def`'s relative media from `from` to `to`, refused
    /// when a path could leave its data folder.
    pub(crate) fn new(def: &MachineDef, from: PathBuf, to: PathBuf) -> Result<Self, String> {
        let media = relative_media(def);
        media
            .files
            .iter()
            .chain(&media.folders)
            .try_for_each(|path| check_inside(path))?;
        Ok(Self { from, to, media })
    }

    /// How many bytes the copy writes: what is there of it now.
    pub(crate) fn bytes(&self) -> u64 {
        let files = self
            .media
            .files
            .iter()
            .filter_map(|file| fs::metadata(self.from.join(file)).ok())
            .map(|meta| meta.len());
        let folders = self
            .media
            .folders
            .iter()
            .map(|folder| tree_bytes(&self.from.join(folder)));
        files.chain(folders).sum()
    }

    /// Whether the copy is large enough to run on a thread of its own.
    pub(crate) fn in_background(&self) -> bool {
        self.bytes() > BACKGROUND_COPY_BYTES
    }

    /// Copy everything into [`Self::to`], creating it. A missing file
    /// fails the copy; the caller removes `to`.
    pub(crate) fn run(&self) -> Result<(), String> {
        fs::create_dir_all(&self.to).map_err(|e| path_error(&self.to, &e))?;
        for file in &self.media.files {
            let source = self.from.join(file);
            if !source.is_file() {
                return Err(format!("{file} is missing from {}", self.from.display()));
            }
            copy_file(&source, &self.to.join(file))?;
        }
        for folder in &self.media.folders {
            let source = self.from.join(folder);
            if source.is_dir() {
                copy_tree(&source, &self.to.join(folder))?;
            }
        }
        Ok(())
    }
}

/// Copy `source` to `target`, creating `target`'s parent folders.
fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| path_error(parent, &e))?;
    }
    fs::copy(source, target)
        .map(drop)
        .map_err(|e| path_error(target, &e))
}

/// Copy the folder `source` and everything in it to `target`. A symlink is
/// copied as the file it points at; one to a folder, or a dangling one, is
/// skipped, so a link loop can't make the copy endless.
fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| path_error(target, &e))?;
    let entries = fs::read_dir(source).map_err(|e| path_error(source, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| path_error(source, &e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| path_error(&path, &e))?;
        let destination = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&path, &destination)?;
        } else if file_type.is_file() || path.is_file() {
            copy_file(&path, &destination)?;
        }
    }
    Ok(())
}

/// The bytes of every file under `dir`, following the same rules as
/// [`copy_tree`]. Unreadable entries count as empty.
pub(crate) fn tree_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => tree_bytes(&entry.path()),
            _ => fs::metadata(entry.path())
                .ok()
                .filter(fs::Metadata::is_file)
                .map_or(0, |meta| meta.len()),
        })
        .sum()
}

/// Create a fresh folder under `root` for the first slug from `base` that
/// `taken` refuses and no folder already has, and return the slug.
/// `fs::create_dir` fails when the folder exists, so of two instances
/// claiming at once, only one gets a slug: the other moves on to the next
/// one instead of copying into, and on failure deleting, the same files.
pub(crate) fn claim(
    root: &Path,
    base: &str,
    taken: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    fs::create_dir_all(root).map_err(|e| path_error(root, &e))?;
    // Candidates already present, even where `taken` misses them (a
    // dangling symlink), so the search always moves on.
    let mut lost = Vec::new();
    loop {
        let slug = crate::machine_def::unique_slug(base, &|candidate| {
            lost.iter().any(|lost_slug| lost_slug == candidate) || taken(candidate)
        });
        let dir = root.join(&slug);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(slug),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => lost.push(slug),
            Err(e) => return Err(path_error(&dir, &e)),
        }
    }
}

/// Remove a folder [`claim`] created, with whatever was copied into it.
/// Best effort: what stays only makes its slug count as taken.
pub(crate) fn discard(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
}

/// `error` from an operation on `path`, naming the path.
fn path_error(path: &Path, error: &io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
#[path = "media_test.rs"]
mod tests;
