//! Host jobs behind the `dw` commands. Each blocks on the host filesystem,
//! so it runs on the VM's DriveWire host executor, never inside a Becker
//! register access. A job whose result is not plain bytes leaves it in an
//! [`Outcome`] and returns an empty payload.

use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::super::host::{HostError, HostJob, MAX_HOST_RESPONSE_BYTES};
use super::super::share::{ShareError, ShareImage, ShareOp, ShareReader, ShareSession};

/// Where a job leaves a result that is not bytes.
pub(super) type Outcome<T> = Arc<Mutex<Option<T>>>;

/// One page of a directory listing.
pub(super) struct DirPage {
    /// The normalized directory name, on the first page only.
    pub heading: Option<String>,
    /// `name\n` lines, as [`ShareOp::List`] returns them.
    pub listing: Vec<u8>,
}

/// A file being streamed by `dw server list`. Dropping the last reference
/// closes it and ends its lease.
pub(super) type SharedReader = Arc<Mutex<ShareReader>>;

pub(super) fn take<T>(outcome: &Outcome<T>) -> Option<T> {
    lock(outcome).take()
}

/// Lists entries from `start`; the first page also normalizes the path.
pub(super) fn dir_page(
    session: &ShareSession,
    path: Vec<u8>,
    start: usize,
) -> (HostJob, Outcome<DirPage>) {
    let session = session.clone();
    typed(move || {
        let heading = match start {
            0 => Some(session.display_path(&path)?),
            _ => None,
        };
        let listing = session.execute(ShareOp::List { path, start })?;
        Ok(DirPage { heading, listing })
    })
}

pub(super) fn open_reader(
    session: &ShareSession,
    path: Vec<u8>,
) -> (HostJob, Outcome<ShareReader>) {
    let session = session.clone();
    typed(move || session.open_reader(&path))
}

pub(super) fn open_image(session: &ShareSession, path: Vec<u8>) -> (HostJob, Outcome<ShareImage>) {
    let session = session.clone();
    typed(move || session.open_image(&path))
}

/// Reads the next chunk, up to the host response limit; empty at the end.
pub(super) fn read_chunk(reader: &SharedReader) -> HostJob {
    let reader = Arc::clone(reader);
    Box::new(move |cancellation| {
        if cancellation.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let mut reader = lock(&reader);
        let limit = u64::try_from(MAX_HOST_RESPONSE_BYTES).expect("bounded by a usize");
        let mut bytes = Vec::new();
        (&mut reader.file).take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

fn typed<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ShareError> + Send + 'static,
) -> (HostJob, Outcome<T>) {
    let outcome: Outcome<T> = Arc::default();
    let slot = Arc::clone(&outcome);
    let job: HostJob = Box::new(move |cancellation| {
        if cancellation.is_cancelled() {
            return Err(HostError::Cancelled);
        }
        let value = work().map_err(HostError::Share)?;
        *lock(&slot) = Some(value);
        Ok(Vec::new())
    });
    (job, outcome)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
