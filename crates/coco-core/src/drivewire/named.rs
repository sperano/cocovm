//! DriveWire named objects: `OP_NAMEOBJ_MOUNT` and `OP_NAMEOBJ_CREATE`.
//!
//! A guest sends the opcode, a length byte, and that many name bytes. The
//! server replies with one byte: [`NAMED_FAILURE`] (0), or the drive number
//! now holding the object, which the guest then reads and writes with the
//! ordinary sector opcodes. Mount fails for a missing object and create for
//! an existing one; neither reports why (DriveWire specification
//! `a795310`, "Named Objects"). Names resolve inside the share the VM
//! selected for named objects (`ShareSession::open_named`), on the VM's
//! host executor.
//!
//! Each completed call ends the previous lease, and a successful one starts
//! a lease on the drive it returns. A drive holding an object only ever
//! reaches that object: configured mounts and `dw disk insert` address
//! drives 0–3 only, and objects use [`NAMED_DRIVE_FIRST`]–255. Objects stay
//! mounted after their lease ends, at most [`MAX_NAMED_OBJECTS`] at a time;
//! a new object then replaces the one returned least recently. Drive numbers
//! are handed out counting down from 255 (the DriveWire 4 Java server's
//! `getFreeDriveNo` order) and wrap, so a replaced object's number is not
//! reused until the others have been, and a guest still using it gets
//! "not ready" instead of another object's sectors.
//!
//! Sector I/O to an object differs from a disk image in one way: a final
//! sector that the file only partly covers reads back zero-padded, since an
//! object is a byte collection rather than a disk. Reads wholly past the
//! end still fail. In HDB-DOS mode, requests whose wire drive holds an
//! object bypass the LSN/630 remapping (pyDriveWire `9c85a9f`
//! `_isNamedObjDrive`); HDB-DOS itself sends wire drive 0.
//!
//! Objects survive machine and protocol resets, like configured media.
//! Snapshots record each object's name, drive, and write protection, never
//! its handle: [`DWServer::reattach_named_objects`] reopens them by name
//! after a restore, and an object that cannot be reopened is dropped, so
//! its drive answers "not ready" rather than another object's sectors.

use serde::{Deserialize, Serialize};

use super::command::{Outcome, take_outcome, typed_job};
use super::host::HostError;
use super::protocol::State;
use super::share::{Lease, ShareError, ShareImage};
use super::{DRIVE_COUNT, DWImage, DWServer};

/// Reply byte for a failed named-object call.
pub const NAMED_FAILURE: u8 = 0;

/// Lowest drive number an object can get: the first one after the
/// configured drives.
pub const NAMED_DRIVE_FIRST: u8 = DRIVE_COUNT as u8;

/// Most objects mounted at once, which bounds the host files a guest can
/// hold open this way.
pub const MAX_NAMED_OBJECTS: usize = 8;

/// A mounted object, as settings and status displays describe it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedObjectInfo {
    pub drive: u8,
    /// The name the guest sent.
    pub name: String,
    /// Normalized share path, for example `/games/boot/os9.dsk`.
    pub guest_path: String,
    /// Writes fail: the object's share is read-only.
    pub write_protected: bool,
    /// The latest named-object call returned this object.
    pub leased: bool,
    /// The object has a host handle. Only a restored object waiting for
    /// [`DWServer::reattach_named_objects`] has none.
    pub attached: bool,
}

/// An object [`DWServer::reattach_named_objects`] could not reopen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedRestoreFailure {
    pub drive: u8,
    pub name: String,
    pub error: ShareError,
}

#[derive(Serialize, Deserialize)]
struct NamedObject {
    drive: u8,
    name: String,
    guest_path: String,
    write_protected: bool,
    /// The call count when a call last returned this object; the smallest
    /// is replaced first.
    last_used: u64,
    #[serde(skip)]
    image: Option<DWImage>,
    #[serde(skip)]
    lease: Option<Lease>,
}

/// The objects one server holds. Handles and leases stay out of snapshots.
#[derive(Serialize, Deserialize)]
pub(super) struct NamedObjects {
    objects: Vec<NamedObject>,
    /// The drive the latest successful call returned, until the next call.
    lease: Option<u8>,
    /// Completed calls, for least-recently-returned replacement.
    calls: u64,
    /// The next drive number to try; counts down and wraps.
    cursor: u8,
}

impl Default for NamedObjects {
    fn default() -> Self {
        Self {
            objects: Vec::new(),
            lease: None,
            calls: 0,
            cursor: u8::MAX,
        }
    }
}

impl NamedObjects {
    /// Whether `drive` holds an object, attached or not.
    pub(super) fn holds(&self, drive: u8) -> bool {
        self.objects.iter().any(|object| object.drive == drive)
    }

    pub(super) fn image(&self, drive: usize) -> Option<&DWImage> {
        self.find(drive)?.image.as_ref()
    }

    pub(super) fn image_mut(&mut self, drive: usize) -> Option<&mut DWImage> {
        self.objects
            .iter_mut()
            .find(|object| usize::from(object.drive) == drive)?
            .image
            .as_mut()
    }

    pub(super) fn write_protected(&self, drive: usize) -> bool {
        self.find(drive)
            .is_some_and(|object| object.write_protected)
    }

    /// Closes every object and ends the lease.
    pub(super) fn clear(&mut self) {
        self.objects.clear();
        self.lease = None;
    }

    fn find(&self, drive: usize) -> Option<&NamedObject> {
        self.objects
            .iter()
            .find(|object| usize::from(object.drive) == drive)
    }

    /// Starts a call: the previous lease ends whatever the outcome.
    fn begin_call(&mut self) {
        self.lease = None;
        self.calls += 1;
    }

    /// Mounts `image` for the guest's `name` and leases its drive. The same
    /// host file keeps the drive it already has, with the new handle.
    fn attach(&mut self, name: String, image: ShareImage) -> u8 {
        let calls = self.calls;
        let existing = self.objects.iter().position(|object| {
            object
                .lease
                .as_ref()
                .is_some_and(|lease| lease.same_file(&image.lease))
        });
        let index = match existing {
            Some(index) => index,
            None => {
                self.make_room();
                let drive = self.allocate();
                self.objects.push(NamedObject {
                    drive,
                    name: String::new(),
                    guest_path: String::new(),
                    write_protected: false,
                    last_used: calls,
                    image: None,
                    lease: None,
                });
                self.objects.len() - 1
            }
        };
        let object = &mut self.objects[index];
        object.name = name;
        object.guest_path = image.guest_path;
        object.write_protected = !image.writable;
        object.last_used = calls;
        object.image = Some(DWImage::File(image.file).into_async());
        object.lease = Some(image.lease);
        self.lease = Some(object.drive);
        object.drive
    }

    /// Drops the least recently returned object when the table is full.
    fn make_room(&mut self) {
        while self.objects.len() >= MAX_NAMED_OBJECTS {
            let Some(oldest) = self
                .objects
                .iter()
                .enumerate()
                .min_by_key(|(_, object)| object.last_used)
                .map(|(index, _)| index)
            else {
                return;
            };
            self.objects.remove(oldest);
        }
    }

    /// The next free drive number at or below the cursor, wrapping from
    /// [`NAMED_DRIVE_FIRST`] back to 255. [`Self::make_room`] leaves far
    /// more free numbers than objects, so one pass always finds one.
    fn allocate(&mut self) -> u8 {
        let mut drive = self.cursor.max(NAMED_DRIVE_FIRST);
        for _ in NAMED_DRIVE_FIRST..=u8::MAX {
            if !self.holds(drive) {
                break;
            }
            drive = previous_drive(drive);
        }
        self.cursor = previous_drive(drive);
        drive
    }

    fn info(&self) -> Vec<NamedObjectInfo> {
        self.objects
            .iter()
            .map(|object| NamedObjectInfo {
                drive: object.drive,
                name: object.name.clone(),
                guest_path: object.guest_path.clone(),
                write_protected: object.write_protected,
                leased: self.lease == Some(object.drive),
                attached: object.image.is_some(),
            })
            .collect()
    }
}

fn previous_drive(drive: u8) -> u8 {
    if drive <= NAMED_DRIVE_FIRST {
        u8::MAX
    } else {
        drive - 1
    }
}

impl DWServer {
    /// Mounted named objects, in the order they were first mounted.
    pub fn named_objects(&self) -> Vec<NamedObjectInfo> {
        self.named.info()
    }

    /// Reopens restored objects by name through the current shares. Call it
    /// after a snapshot restore, once [`Self::set_shares`] has installed the
    /// VM's shares; it blocks on the host filesystem. Objects that cannot be
    /// reopened are dropped and returned.
    pub fn reattach_named_objects(&mut self) -> Vec<NamedRestoreFailure> {
        let mut failures = Vec::new();
        let shares = self.shares.clone();
        self.named.objects.retain_mut(|object| {
            if object.image.is_some() {
                return true;
            }
            match shares.open_named(object.name.as_bytes()) {
                Ok(image) => {
                    object.write_protected = !image.writable;
                    object.image = Some(DWImage::File(image.file).into_async());
                    object.lease = Some(image.lease);
                    true
                }
                Err(error) => {
                    failures.push(NamedRestoreFailure {
                        drive: object.drive,
                        name: object.name.clone(),
                        error,
                    });
                    false
                }
            }
        });
        if self
            .named
            .lease
            .is_some_and(|drive| !self.named.holds(drive))
        {
            self.named.lease = None;
        }
        failures
    }

    /// The opcode byte arrived; the length byte follows.
    pub(super) fn begin_named(&mut self, create: bool) {
        self.state = State::AwaitNameLength { create };
    }

    /// [`State::AwaitNameLength`].
    pub(super) fn feed_name_length(&mut self, create: bool, len: u8) {
        if len == 0 {
            self.complete_named(create, Vec::new());
        } else {
            self.state = State::AwaitName {
                create,
                len,
                buf: Vec::with_capacity(usize::from(len)),
            };
        }
    }

    /// [`State::AwaitName`]: accumulate the name, then resolve it.
    pub(super) fn feed_name(&mut self, create: bool, len: u8, mut buf: Vec<u8>, byte: u8) {
        buf.push(byte);
        if buf.len() == usize::from(len) {
            self.complete_named(create, buf);
        } else {
            self.state = State::AwaitName { create, len, buf };
        }
    }

    /// A whole request arrived: end the lease and resolve the name on the
    /// host executor. An empty name fails at once.
    fn complete_named(&mut self, create: bool, name: Vec<u8>) {
        self.named.begin_call();
        if name.is_empty() {
            self.reply.push_back(NAMED_FAILURE);
            return;
        }
        let session = self.shares.clone();
        let (job, outcome) = typed_job(move || {
            let image = if create {
                session.create_named(&name)?
            } else {
                session.open_named(&name)?
            };
            let name = String::from_utf8(name).map_err(|_| ShareError::InvalidPath)?;
            Ok((name, image))
        });
        self.state = State::AwaitHostNamed;
        self.begin_named_request(job, outcome);
    }

    /// The host resolved (or failed to resolve) the pending request.
    pub(super) fn finish_named(
        &mut self,
        result: Result<Vec<u8>, HostError>,
        outcome: &Outcome<(String, ShareImage)>,
    ) {
        let attached = result
            .ok()
            .and_then(|_| take_outcome(outcome))
            .map(|(name, image)| self.named.attach(name, image));
        self.reply.push_back(attached.unwrap_or(NAMED_FAILURE));
    }
}

#[cfg(test)]
#[path = "named_test.rs"]
mod tests;
