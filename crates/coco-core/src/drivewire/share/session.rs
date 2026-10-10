//! One VM's share session: its current directory, open handles, and the
//! operations guest services run against them.
//!
//! Operations block on the host filesystem, so services run them as host
//! jobs ([`ShareSession::job`]) on the VM's DriveWire executor rather than
//! inside a Becker register write. The emulation thread never waits on the
//! session lock: [`ShareSession::reset`] swaps in fresh state, and a job
//! still running keeps the old state alive until it returns. Its handles
//! close and its leases end then.
//!
//! An open handle refers to the file it opened, not to its name. Renaming or
//! deleting the file, or removing the share folder, leaves the handle usable
//! where the host allows it (Unix does). New requests resolve the current
//! name and fail if it is gone.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::access::{AccessMode, AccessRegistry, Lease, LeaseOwner};
use super::path::{self, Entry, GuestPath};
use super::{ShareAccess, ShareError, ShareTable};
use crate::drivewire::host::{HostError, HostJob, MAX_HOST_RESPONSE_BYTES};

/// Handles one session can hold open at once.
pub const MAX_OPEN_HANDLES: usize = 8;

const LINE_END: u8 = b'\n';
const DIR_MARKER: u8 = b'/';

/// A session-local file handle, carried on the wire as one byte from 1 to
/// [`MAX_OPEN_HANDLES`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShareHandle(u8);

impl ShareHandle {
    pub fn from_byte(byte: u8) -> Self {
        Self(byte)
    }

    pub fn byte(self) -> u8 {
        self.0
    }

    fn slot(self) -> Option<usize> {
        usize::from(self.0)
            .checked_sub(1)
            .filter(|&slot| slot < MAX_OPEN_HANDLES)
    }
}

/// A share operation and the payload its successful reply carries. Paths
/// are guest bytes (see [`path`]); relative paths start at the session's
/// current directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShareOp {
    /// Lists a directory, sorted by name, starting at entry `start`. Each
    /// entry is one `name\n` line, `name/\n` for a directory. The payload
    /// stops before the response limit; an empty payload means no entries
    /// remain.
    List { path: Vec<u8>, start: usize },
    /// Sets the current directory. Empty payload.
    ChangeDir { path: Vec<u8> },
    /// Opens a file for reading. Payload: the handle byte.
    OpenRead { path: Vec<u8> },
    /// Creates a file, or truncates an existing one, for writing. Payload:
    /// the handle byte.
    Create { path: Vec<u8> },
    /// Reads up to `max` bytes, clamped to the response limit. An empty
    /// payload is end of file.
    Read { handle: ShareHandle, max: usize },
    /// Writes at most the response limit's worth of bytes. Empty payload.
    Write { handle: ShareHandle, data: Vec<u8> },
    /// Closes a handle and ends its lease. Empty payload.
    Close { handle: ShareHandle },
}

/// A disk image opened through a share for a guest mount. Read-only shares
/// yield read-only images, whose writes fail with a DriveWire write error.
#[derive(Debug)]
pub struct ShareImage {
    pub file: File,
    pub writable: bool,
    /// Keep for as long as the image is mounted.
    pub lease: Lease,
}

/// What settings show about a running session.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShareStatus {
    pub cwd: String,
    pub open_handles: usize,
    /// Latest failed operation; a later success does not clear it.
    pub last_error: Option<ShareError>,
}

struct OpenFile {
    file: File,
    writable: bool,
    _lease: Lease,
}

#[derive(Default)]
struct State {
    cwd: GuestPath,
    handles: [Option<OpenFile>; MAX_OPEN_HANDLES],
}

/// One VM's share session. Clones refer to the same session.
#[derive(Clone)]
pub struct ShareSession {
    table: Arc<ShareTable>,
    owner: LeaseOwner,
    registry: Arc<AccessRegistry>,
    state: Arc<Mutex<State>>,
    status: Arc<Mutex<ShareStatus>>,
}

impl Default for ShareSession {
    /// No shares, owned by a fresh [`LeaseOwner`].
    fn default() -> Self {
        Self::new(
            ShareTable::default(),
            LeaseOwner::new(),
            AccessRegistry::global(),
        )
    }
}

impl ShareSession {
    pub fn new(table: ShareTable, owner: LeaseOwner, registry: Arc<AccessRegistry>) -> Self {
        Self {
            table: Arc::new(table),
            owner,
            registry,
            state: Arc::default(),
            status: Arc::new(Mutex::new(fresh_status())),
        }
    }

    pub fn table(&self) -> &ShareTable {
        &self.table
    }

    pub fn owner(&self) -> LeaseOwner {
        self.owner
    }

    pub fn status(&self) -> ShareStatus {
        lock(&self.status).clone()
    }

    /// Starts over at the top level with no handles, without waiting for a
    /// running job (see the module doc).
    pub fn reset(&mut self) {
        self.state = Arc::default();
        self.status = Arc::new(Mutex::new(fresh_status()));
    }

    /// Runs `op` on the calling thread. Guest services call it from a host
    /// job; see [`Self::job`].
    pub fn execute(&self, op: ShareOp) -> Result<Vec<u8>, ShareError> {
        self.execute_with(op, &|| false)
    }

    /// Wraps `op` for the DriveWire host executor.
    pub fn job(&self, op: ShareOp) -> HostJob {
        let session = self.clone();
        Box::new(move |cancellation| {
            session
                .execute_with(op, &|| cancellation.is_cancelled())
                .map_err(HostError::Share)
        })
    }

    /// Opens a disk image for a guest mount, leased for writing when the
    /// share is read/write and for reading otherwise. Blocks on the host
    /// filesystem and the session lock, so call it from a host job, never
    /// inside a Becker register access.
    pub fn open_image(&self, path: &[u8]) -> Result<ShareImage, ShareError> {
        let state = lock(&self.state);
        let result = self.open_image_in(&state, path);
        self.record(&state, result.as_ref().err().copied());
        result
    }

    fn execute_with(
        &self,
        op: ShareOp,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, ShareError> {
        let mut state = lock(&self.state);
        let result = self.dispatch(&mut state, op, is_cancelled);
        self.record(&state, result.as_ref().err().copied());
        result
    }

    fn dispatch(
        &self,
        state: &mut State,
        op: ShareOp,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, ShareError> {
        match op {
            ShareOp::List { path, start } => {
                let guest = GuestPath::parse(&path, &state.cwd, &self.table)?;
                let entries = path::list(&guest, &self.table, is_cancelled)?;
                Ok(encode_listing(&entries, start))
            }
            ShareOp::ChangeDir { path } => self.change_dir(state, &path).map(|()| Vec::new()),
            ShareOp::OpenRead { path } => self.open_read(state, &path),
            ShareOp::Create { path } => self.create(state, &path),
            ShareOp::Read { handle, max } => read(open_file(state, handle)?, max),
            ShareOp::Write { handle, data } => write(state, handle, &data),
            ShareOp::Close { handle } => {
                let slot = handle.slot().ok_or(ShareError::BadHandle)?;
                state.handles[slot].take().ok_or(ShareError::BadHandle)?;
                Ok(Vec::new())
            }
        }
    }

    fn change_dir(&self, state: &mut State, path: &[u8]) -> Result<(), ShareError> {
        let guest = GuestPath::parse(path, &state.cwd, &self.table)?;
        if !guest.is_top() {
            let resolved = path::resolve_existing(&guest, &self.table)?;
            if !resolved.host.is_dir() {
                return Err(ShareError::NotADirectory);
            }
        }
        state.cwd = guest;
        Ok(())
    }

    fn open_read(&self, state: &mut State, path: &[u8]) -> Result<Vec<u8>, ShareError> {
        let slot = free_slot(state)?;
        let guest = self.file_path(state, path)?;
        let resolved = path::resolve_existing(&guest, &self.table)?;
        let file = File::open(&resolved.host)?;
        reject_directory(&file)?;
        let lease = self
            .registry
            .acquire(&file, &resolved.host, self.owner, AccessMode::Read)?;
        Ok(store(state, slot, file, false, lease))
    }

    fn create(&self, state: &mut State, path: &[u8]) -> Result<Vec<u8>, ShareError> {
        let slot = free_slot(state)?;
        let guest = self.file_path(state, path)?;
        let resolved = path::resolve_new(&guest, &self.table)?;
        if resolved.access == ShareAccess::ReadOnly {
            return Err(ShareError::ReadOnly);
        }
        // Truncate only once the lease proves no other VM is using the file.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&resolved.host)?;
        let lease = self
            .registry
            .acquire(&file, &resolved.host, self.owner, AccessMode::Write)?;
        file.set_len(0)?;
        Ok(store(state, slot, file, true, lease))
    }

    fn open_image_in(&self, state: &State, path: &[u8]) -> Result<ShareImage, ShareError> {
        let guest = self.file_path(state, path)?;
        let resolved = path::resolve_existing(&guest, &self.table)?;
        let writable = resolved.access == ShareAccess::ReadWrite;
        let file = OpenOptions::new()
            .read(true)
            .write(writable)
            .open(&resolved.host)?;
        reject_directory(&file)?;
        let mode = if writable {
            AccessMode::Write
        } else {
            AccessMode::Read
        };
        let lease = self
            .registry
            .acquire(&file, &resolved.host, self.owner, mode)?;
        Ok(ShareImage {
            file,
            writable,
            lease,
        })
    }

    /// Parses a path that must name something below a share root.
    fn file_path(&self, state: &State, path: &[u8]) -> Result<GuestPath, ShareError> {
        if path.is_empty() {
            return Err(ShareError::InvalidPath);
        }
        let guest = GuestPath::parse(path, &state.cwd, &self.table)?;
        if guest.components().is_empty() {
            return Err(ShareError::IsADirectory);
        }
        Ok(guest)
    }

    fn record(&self, state: &State, error: Option<ShareError>) {
        let mut status = lock(&self.status);
        status.cwd = state.cwd.to_string();
        status.open_handles = state.handles.iter().flatten().count();
        if error.is_some() {
            status.last_error = error;
        }
    }
}

fn fresh_status() -> ShareStatus {
    ShareStatus {
        cwd: GuestPath::top().to_string(),
        ..ShareStatus::default()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panicking job leaves at most one handle half-opened, which is still
    // a consistent table.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn free_slot(state: &State) -> Result<usize, ShareError> {
    state
        .handles
        .iter()
        .position(Option::is_none)
        .ok_or(ShareError::TooManyHandles)
}

fn store(state: &mut State, slot: usize, file: File, writable: bool, lease: Lease) -> Vec<u8> {
    state.handles[slot] = Some(OpenFile {
        file,
        writable,
        _lease: lease,
    });
    let handle = u8::try_from(slot + 1).expect("MAX_OPEN_HANDLES fits a byte");
    vec![handle]
}

fn open_file(state: &mut State, handle: ShareHandle) -> Result<&mut OpenFile, ShareError> {
    let slot = handle.slot().ok_or(ShareError::BadHandle)?;
    state.handles[slot].as_mut().ok_or(ShareError::BadHandle)
}

fn reject_directory(file: &File) -> Result<(), ShareError> {
    let metadata = file.metadata()?;
    if metadata.is_dir() {
        Err(ShareError::IsADirectory)
    } else {
        Ok(())
    }
}

fn read(open: &mut OpenFile, max: usize) -> Result<Vec<u8>, ShareError> {
    let limit = u64::try_from(max.min(MAX_HOST_RESPONSE_BYTES)).expect("bounded by a usize");
    let mut bytes = Vec::new();
    (&mut open.file).take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn write(state: &mut State, handle: ShareHandle, data: &[u8]) -> Result<Vec<u8>, ShareError> {
    if data.len() > MAX_HOST_RESPONSE_BYTES {
        return Err(ShareError::TooLarge);
    }
    let open = open_file(state, handle)?;
    if !open.writable {
        return Err(ShareError::ReadOnly);
    }
    open.file.write_all(data)?;
    Ok(Vec::new())
}

fn encode_listing(entries: &[Entry], start: usize) -> Vec<u8> {
    let mut payload = Vec::new();
    for entry in entries.iter().skip(start) {
        let line_len = entry.name.len() + usize::from(entry.is_dir) + 1;
        if payload.len() + line_len > MAX_HOST_RESPONSE_BYTES {
            break;
        }
        payload.extend_from_slice(entry.name.as_bytes());
        if entry.is_dir {
            payload.push(DIR_MARKER);
        }
        payload.push(LINE_END);
    }
    payload
}

#[cfg(test)]
#[path = "session_test.rs"]
mod tests;
