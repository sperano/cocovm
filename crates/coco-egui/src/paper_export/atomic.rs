use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{ExportError, message, path_error};

const TEMP_CREATE_ATTEMPTS: usize = 32;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_atomic_file(
    destination: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> Result<(), ExportError> {
    let (file, temporary_path) = create_temporary_file(destination)?;
    let guard = TemporaryFile::new(temporary_path);
    let mut writer = BufWriter::new(file);
    write(&mut writer).map_err(|error| path_error("write", destination, error))?;
    writer
        .flush()
        .map_err(|error| path_error("flush", destination, error))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| path_error("sync", destination, error))?;
    drop(writer);
    guard.commit(destination)
}

pub(super) fn write_file(
    destination: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
) -> Result<(), ExportError> {
    let file =
        File::create(destination).map_err(|error| path_error("create", destination, error))?;
    let mut writer = BufWriter::new(file);
    write(&mut writer).map_err(|error| path_error("write", destination, error))?;
    writer
        .flush()
        .map_err(|error| path_error("flush", destination, error))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| path_error("sync", destination, error))
}

fn create_temporary_file(destination: &Path) -> Result<(File, PathBuf), ExportError> {
    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let candidate = temporary_sibling(destination);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((file, candidate)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(path_error("create", &candidate, error)),
        }
    }
    Err(message(format!(
        "could not allocate a temporary file beside {}",
        destination.display()
    )))
}

fn temporary_sibling(destination: &Path) -> PathBuf {
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = OsString::from(".");
    name.push(
        destination
            .file_name()
            .unwrap_or_else(|| OsStr::new("printer-export")),
    );
    name.push(format!(".{}.{sequence}.tmp", std::process::id()));
    destination.with_file_name(name)
}

struct TemporaryFile {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(mut self, destination: &Path) -> Result<(), ExportError> {
        fs::rename(&self.path, destination)
            .map_err(|error| path_error("replace", destination, error))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) struct TemporaryDirectory {
    path: PathBuf,
    committed: bool,
}

impl TemporaryDirectory {
    pub(super) fn create(destination: &Path) -> Result<Self, ExportError> {
        if destination.exists() {
            return Err(message(format!(
                "{} already exists; choose a new folder name for page PNGs",
                destination.display()
            )));
        }
        for _ in 0..TEMP_CREATE_ATTEMPTS {
            let path = temporary_sibling(destination);
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        committed: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(path_error("create", &path, error)),
            }
        }
        Err(message(format!(
            "could not allocate a temporary folder beside {}",
            destination.display()
        )))
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn commit(mut self, destination: &Path) -> Result<(), ExportError> {
        fs::rename(&self.path, destination)
            .map_err(|error| path_error("create", destination, error))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
