//! Where finished DriveWire print jobs go, and the folder writer that puts
//! them on the host without blocking emulation.
//!
//! A folder job becomes one new file, written by a worker thread that each
//! [`FolderWriter`] owns. The worker is not the cancellable DriveWire host
//! executor: a job the guest has flushed is guest output already, so resets
//! never cancel it, and nothing could tell whether a cancelled write had
//! already reached the disk. A file name is claimed with `create_new`, so
//! two VMs printing into one folder never overwrite each other, and a write
//! that fails removes its partial file, so a retry cannot leave a duplicate.
//! When that removal fails too, the job keeps the file's path and its next
//! write into that folder removes it first, or fails again.

use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::printer::to_host_text;

use super::{JobEnd, PrintJob};

/// Highest numeric suffix tried after a file name is taken.
const NAME_COLLISION_ATTEMPTS: u32 = 1000;

/// What happens to a job when the guest flushes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PrintOutput {
    /// Jobs are discarded and counted. The default, so enabling DriveWire
    /// never prints anywhere the user did not choose.
    #[default]
    Off,
    /// One new file per job in `dir`.
    Folder { dir: PathBuf, format: PrintFormat },
    /// The VM's own printer port: the printer or capture file attached to
    /// the bit-banger, through [`crate::bitbanger::BitBanger::print_bytes`].
    PrinterPort,
}

/// File format of a [`PrintOutput::Folder`] job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrintFormat {
    /// Host text: CR LF and lone CR become LF (OS-9 ends lines with CR).
    #[default]
    Text,
    /// The guest's bytes unchanged, as the DriveWire 4 Java text printer
    /// writes them (`DWVPrinterTEXT.flush`).
    Raw,
}

impl PrintFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Text => "txt",
            Self::Raw => "prn",
        }
    }

    fn encode(self, bytes: &[u8]) -> Cow<'_, [u8]> {
        match self {
            Self::Text => Cow::Owned(to_host_text(bytes)),
            Self::Raw => Cow::Borrowed(bytes),
        }
    }
}

/// File name stem for `job`: its flush time, job number, and why it ended.
pub(super) fn file_stem(job: &PrintJob) -> String {
    let t = job.stamp;
    let mut stem = format!(
        "print-{:04}{:02}{:02}-{:02}{:02}{:02}-job{:04}",
        t.year, t.month, t.day, t.hour, t.minute, t.second, job.number
    );
    if job.part > 1 || job.end == JobEnd::Split {
        stem.push_str(&format!("-part{}", job.part));
    }
    if job.end == JobEnd::Unterminated {
        stem.push_str("-incomplete");
    }
    stem
}

/// Writes `job` into a new file in `dir`, creating `dir` itself (not its
/// parents) when missing. Returns the file written. A failed write that
/// cannot remove its partial file records it in [`PrintJob::leftover`].
pub(super) fn write_job(
    job: &mut PrintJob,
    dir: &Path,
    format: PrintFormat,
) -> io::Result<PathBuf> {
    remove_leftover(job, dir)?;
    match fs::create_dir(dir) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(error),
        _ => {}
    }
    let stem = file_stem(job);
    let extension = format.extension();
    let bytes = format.encode(&job.bytes);
    for attempt in 0..=NAME_COLLISION_ATTEMPTS {
        let name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{attempt}.{extension}")
        };
        let path = dir.join(name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            return Err(match fs::remove_file(&path) {
                Ok(()) => error,
                Err(remove_error) => {
                    let message = format!(
                        "{error}; partial file {} not removed: {remove_error}",
                        path.display()
                    );
                    job.leftover = Some(path);
                    io::Error::new(error.kind(), message)
                }
            });
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free file name for {stem}"),
    ))
}

/// Removes the partial file an earlier failed write of `job` left behind.
/// While it is still in `dir`, writing the job there again would leave a
/// second, complete file beside it, so the write fails instead. A file left
/// in another folder cannot duplicate a job written to `dir`; that folder
/// keeps it, as the earlier error reported.
fn remove_leftover(job: &mut PrintJob, dir: &Path) -> io::Result<()> {
    let Some(leftover) = job.leftover.take() else {
        return Ok(());
    };
    match fs::remove_file(&leftover) {
        Err(error) if error.kind() != io::ErrorKind::NotFound && leftover.parent() == Some(dir) => {
            let message = format!("partial file {} not removed: {error}", leftover.display());
            job.leftover = Some(leftover);
            Err(io::Error::new(error.kind(), message))
        }
        _ => Ok(()),
    }
}

struct WriteRequest {
    job: PrintJob,
    dir: PathBuf,
    format: PrintFormat,
}

/// A folder write's outcome. A failed job comes back for another attempt,
/// with the folder and format it failed in.
pub(super) enum WriteResult {
    Written(PathBuf),
    Failed {
        job: PrintJob,
        dir: PathBuf,
        format: PrintFormat,
        error: String,
    },
}

/// One VM's folder writer thread. Dropping it lets the thread finish the
/// jobs already sent and exit; failures it can no longer report go to
/// standard error.
pub(super) struct FolderWriter {
    requests: mpsc::Sender<WriteRequest>,
    results: mpsc::Receiver<WriteResult>,
}

impl FolderWriter {
    pub(super) fn spawn() -> io::Result<Self> {
        let (requests, requests_rx) = mpsc::channel::<WriteRequest>();
        let (results_tx, results) = mpsc::channel();
        thread::Builder::new()
            .name("cocovm-drivewire-printer".into())
            .spawn(move || {
                for request in requests_rx {
                    let result = run(request);
                    if let Err(mpsc::SendError(WriteResult::Failed { job, error, .. })) =
                        results_tx.send(result)
                    {
                        eprintln!("DriveWire printer: job {} not written: {error}", job.number);
                    }
                }
            })?;
        Ok(Self { requests, results })
    }

    /// Queues `job`; it comes back as a failure if the thread is gone.
    pub(super) fn send(
        &self,
        job: PrintJob,
        dir: PathBuf,
        format: PrintFormat,
    ) -> Result<(), PrintJob> {
        self.requests
            .send(WriteRequest { job, dir, format })
            .map_err(|mpsc::SendError(request)| request.job)
    }

    pub(super) fn try_result(&self) -> Option<WriteResult> {
        self.results.try_recv().ok()
    }
}

fn run(mut request: WriteRequest) -> WriteResult {
    match write_job(&mut request.job, &request.dir, request.format) {
        Ok(path) => WriteResult::Written(path),
        Err(error) => WriteResult::Failed {
            error: format!("{}: {error}", request.dir.display()),
            job: request.job,
            dir: request.dir,
            format: request.format,
        },
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
mod tests;
