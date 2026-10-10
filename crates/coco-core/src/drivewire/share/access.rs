//! Cross-VM access rules for host files.
//!
//! Every VM in the process leases the host files it opens through shares and
//! the disk images it mounts. Many VMs may read a file at once; a VM that
//! writes it excludes every other VM, reading or writing. Leases of the same
//! owner never conflict, so one VM can open a file more than once. This is the
//! defined concurrent-write behavior: concurrent writes from different VMs are
//! refused rather than interleaved.
//!
//! Files are identified by device and inode on Unix, so hard links and
//! different spellings of one path share a lease. Other platforms use the
//! canonical path, where hard links are distinct files. Processes outside
//! CoCoVM are not coordinated.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use super::ShareError;

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
static GLOBAL: LazyLock<Arc<AccessRegistry>> = LazyLock::new(|| Arc::new(AccessRegistry::new()));

/// Identity of a lease holder, normally one VM.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LeaseOwner(u64);

impl LeaseOwner {
    /// A process-unique owner.
    pub fn new() -> Self {
        Self(NEXT_OWNER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for LeaseOwner {
    fn default() -> Self {
        Self::new()
    }
}

/// How a lease holder uses a file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Read,
    Write,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum FileKey {
    #[cfg(unix)]
    Inode { device: u64, inode: u64 },
    #[cfg(not(unix))]
    Path(PathBuf),
}

struct Holder {
    id: u64,
    owner: LeaseOwner,
    mode: AccessMode,
}

#[derive(Default)]
struct Leases {
    next_id: u64,
    files: HashMap<FileKey, Vec<Holder>>,
}

/// The process's table of leased host files.
#[derive(Default)]
pub struct AccessRegistry {
    leases: Mutex<Leases>,
}

impl AccessRegistry {
    /// A private registry, for tests and embedders with their own scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry every VM in this process shares.
    pub fn global() -> Arc<Self> {
        Arc::clone(&GLOBAL)
    }

    /// Leases the already-open `file`, found at `path`. Fails with
    /// [`ShareError::Busy`] when another owner's lease conflicts.
    pub fn acquire(
        self: &Arc<Self>,
        file: &File,
        path: &Path,
        owner: LeaseOwner,
        mode: AccessMode,
    ) -> Result<Lease, ShareError> {
        let key = file_key(file, path)?;
        let mut leases = self.lock();
        let holders = leases.files.get(&key).map_or(&[][..], Vec::as_slice);
        let conflict = holders.iter().any(|holder| {
            holder.owner != owner && (mode == AccessMode::Write || holder.mode == AccessMode::Write)
        });
        if conflict {
            return Err(ShareError::Busy);
        }
        let id = leases.next_id;
        leases.next_id += 1;
        leases
            .files
            .entry(key.clone())
            .or_default()
            .push(Holder { id, owner, mode });
        Ok(Lease {
            registry: Arc::clone(self),
            key,
            id,
        })
    }

    /// Number of files with at least one lease.
    pub fn leased_files(&self) -> usize {
        self.lock().files.len()
    }

    fn release(&self, key: &FileKey, id: u64) {
        let mut leases = self.lock();
        if let Some(holders) = leases.files.get_mut(key) {
            holders.retain(|holder| holder.id != id);
            if holders.is_empty() {
                leases.files.remove(key);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Leases> {
        // The table stays consistent across a panic: each update is one push or retain.
        self.leases.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A held lease; dropping it releases the file.
pub struct Lease {
    registry: Arc<AccessRegistry>,
    key: FileKey,
    id: u64,
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Lease")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.registry.release(&self.key, self.id);
    }
}

#[cfg(unix)]
fn file_key(file: &File, _path: &Path) -> io::Result<FileKey> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok(FileKey::Inode {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_key(_file: &File, path: &Path) -> io::Result<FileKey> {
    std::fs::canonicalize(path).map(FileKey::Path)
}

#[cfg(test)]
#[path = "access_test.rs"]
mod tests;
