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

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use super::{JobEnd, PrintJob};

/// Highest numeric suffix tried after a file name is taken.
const NAME_COLLISION_ATTEMPTS: u32 = 1000;

const CR: u8 = b'\r';
const LF: u8 = b'\n';

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

    fn encode(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Text => to_host_text(bytes),
            Self::Raw => bytes.to_vec(),
        }
    }
}

/// CR LF and lone CR become LF; every other byte is kept.
pub(super) fn to_host_text(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len());
    let mut after_cr = false;
    for &byte in bytes {
        match byte {
            CR => text.push(LF),
            LF if after_cr => {}
            _ => text.push(byte),
        }
        after_cr = byte == CR;
    }
    text
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
/// parents) when missing. Returns the file written.
pub(super) fn write_job(job: &PrintJob, dir: &Path, format: PrintFormat) -> io::Result<PathBuf> {
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
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("no free file name for {stem}"),
    ))
}

struct WriteRequest {
    job: PrintJob,
    dir: PathBuf,
    format: PrintFormat,
}

/// A folder write's outcome. A failed job comes back for another attempt.
pub(super) enum WriteResult {
    Written(PathBuf),
    Failed { job: PrintJob, error: String },
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
                    if let Err(mpsc::SendError(WriteResult::Failed { job, error })) =
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

fn run(request: WriteRequest) -> WriteResult {
    match write_job(&request.job, &request.dir, request.format) {
        Ok(path) => WriteResult::Written(path),
        Err(error) => WriteResult::Failed {
            error: format!("{}: {error}", request.dir.display()),
            job: request.job,
        },
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
mod tests;
