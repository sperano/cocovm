//! The NitrOS-9 `dw` command service on DriveWire virtual channels:
//! `dw server dir`, `dw server list`, and `dw disk show/insert/eject`.
//!
//! The guest's `dw` utility opens a channel, writes `dw` and its argument
//! line ending in CR, prints the reply, and exits when the channel hangs up
//! (`level1/cmds/dw.as`, NitrOS-9 `0c9940f`). Like the DriveWire 4 Java
//! server's command thread (`DWUtilDWThread`, drivewire4 `4e57ffe`), this
//! service answers each such session once: the status line (see [`reply`]),
//! then the payload, then a hangup the guest sees after reading every byte.
//!
//! Paths resolve through the VM's host shares (`drivewire::share`); host
//! work runs as jobs on the VM's host executor. Replies stream: a command
//! queues at most one host chunk beyond what the channel holds, and asks
//! for the next chunk only once the guest has drained the last one, so a
//! large file never sits in memory whole. File bytes pass through unchanged.

mod jobs;
mod line;
mod parse;
mod reply;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::channel::{CHANNEL_COUNT, ChannelHandle};
use super::host::{HostError, HostJob, RequestId, SubmitError};
use super::share::{ShareError, ShareImage, ShareReader, ShareSession};
use super::{DRIVE_COUNT, DWServer};
use jobs::{DirPage, Outcome, SharedReader};
use line::Line;
use parse::{Command, Failure, code};

/// What the service is doing on one channel.
#[derive(Default)]
enum Slot {
    #[default]
    Idle,
    /// A guest session whose first line has not arrived yet.
    Watching(ChannelHandle),
    /// A session the service leaves alone: its first line was not a `dw`
    /// command, or its command has finished.
    Passed(ChannelHandle),
    Active(Box<Active>),
}

impl Slot {
    fn handle(&self) -> Option<ChannelHandle> {
        match self {
            Self::Idle => None,
            Self::Watching(handle) | Self::Passed(handle) => Some(*handle),
            Self::Active(active) => Some(active.handle),
        }
    }
}

/// One command in progress.
struct Active {
    handle: ChannelHandle,
    /// Reply bytes the channel has not accepted yet.
    output: VecDeque<u8>,
    /// The status line has been queued.
    replied: bool,
    task: Task,
    /// The host job in flight.
    request: Option<RequestId>,
}

/// The host work left before the hangup.
enum Task {
    /// Hang up once the output is queued.
    Finish,
    Dir {
        path: Vec<u8>,
        next: usize,
        page: Option<Outcome<DirPage>>,
    },
    Open {
        path: Vec<u8>,
        reader: Option<Outcome<ShareReader>>,
    },
    Read(SharedReader),
    Insert {
        drive: usize,
        path: Vec<u8>,
        image: Option<Outcome<ShareImage>>,
    },
}

impl Task {
    /// The next host job, keeping its outcome; `None` means hang up.
    fn next_job(&mut self, session: &ShareSession) -> Option<HostJob> {
        Some(match self {
            Self::Finish => return None,
            Self::Dir { path, next, page } => {
                let (job, outcome) = jobs::dir_page(session, path.clone(), *next);
                *page = Some(outcome);
                job
            }
            Self::Open { path, reader } => {
                let (job, outcome) = jobs::open_reader(session, path.clone());
                *reader = Some(outcome);
                job
            }
            Self::Read(reader) => jobs::read_chunk(reader),
            Self::Insert { path, image, .. } => {
                let (job, outcome) = jobs::open_image(session, path.clone());
                *image = Some(outcome);
                job
            }
        })
    }
}

impl Active {
    fn new(handle: ChannelHandle, task: Task) -> Self {
        Self {
            handle,
            output: VecDeque::new(),
            replied: false,
            task,
            request: None,
        }
    }

    fn reply_ok(&mut self, payload: &[u8]) {
        self.output.extend(reply::ok(payload));
        self.replied = true;
    }

    /// Reports `failure` unless the reply has started, then ends the
    /// command. A failure mid-stream can only cut the payload short.
    fn reply_fail(&mut self, failure: &Failure) {
        if !self.replied {
            self.output.extend(reply::fail(failure));
            self.replied = true;
        }
        self.task = Task::Finish;
    }

    /// Takes a finished job's result. A successful insert returns the image
    /// for the server to mount.
    fn absorb(&mut self, result: Result<Vec<u8>, HostError>) -> Option<(usize, ShareImage)> {
        let failed = |error: HostError, path: &[u8]| reply::share_failure(share_error(error), path);
        match std::mem::replace(&mut self.task, Task::Finish) {
            Task::Finish => {}
            Task::Dir { path, next, page } => match outcome(result, page.as_ref()) {
                Ok(page) => self.absorb_page(path, next, page),
                Err(error) => self.reply_fail(&failed(error, &path)),
            },
            Task::Open { path, reader } => match outcome(result, reader.as_ref()) {
                Ok(reader) => {
                    self.reply_ok(&[]);
                    self.task = Task::Read(Arc::new(Mutex::new(reader)));
                }
                Err(error) => self.reply_fail(&failed(error, &path)),
            },
            Task::Read(reader) => match result {
                Ok(bytes) if !bytes.is_empty() => {
                    self.output.extend(bytes);
                    self.task = Task::Read(reader);
                }
                _ => {}
            },
            Task::Insert { drive, path, image } => match outcome(result, image.as_ref()) {
                Ok(image) => {
                    self.reply_ok(format!("Disk inserted in drive {drive}.\r\n").as_bytes());
                    return Some((drive, image));
                }
                Err(error) => self.reply_fail(&reply::insert_failure(share_error(error), &path)),
            },
        }
        None
    }

    fn absorb_page(&mut self, path: Vec<u8>, next: usize, page: DirPage) {
        if let Some(heading) = page.heading {
            self.reply_ok(&reply::dir_heading(&heading));
        }
        let (lines, entries) = reply::dir_lines(&page.listing);
        self.output.extend(lines);
        if entries > 0 {
            self.task = Task::Dir {
                path,
                next: next + entries,
                page: None,
            };
        }
    }
}

/// A job's typed result, or why there is none.
fn outcome<T>(
    result: Result<Vec<u8>, HostError>,
    slot: Option<&Outcome<T>>,
) -> Result<T, HostError> {
    result?;
    slot.and_then(jobs::take)
        .ok_or(HostError::Io(std::io::ErrorKind::Other))
}

/// The guest-facing share error behind a failed job.
fn share_error(error: HostError) -> ShareError {
    match error {
        HostError::Share(error) => error,
        HostError::Io(kind) => ShareError::Io(kind),
        HostError::Cancelled | HostError::Panicked | HostError::ResponseTooLarge { .. } => {
            ShareError::Io(std::io::ErrorKind::Other)
        }
    }
}

/// Command state for every channel. Not part of snapshots.
#[derive(Default)]
pub(super) struct Commands {
    slots: [Slot; CHANNEL_COUNT],
    /// Channel activity at the last scan; `None` forces a scan.
    seen_activity: Option<u64>,
    /// Scan again even without channel activity: a job finished or a
    /// submission must be retried.
    wake: bool,
}

impl Commands {
    /// The channel whose command submitted `id`.
    pub(super) fn owner(&self, id: RequestId) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| matches!(slot, Slot::Active(active) if active.request == Some(id)))
    }
}

impl DWServer {
    /// Advances every channel's command; called from [`Self::poll_host`].
    /// Skips the scan while no channel or job changed.
    pub(super) fn step_commands(&mut self) {
        let activity = self.channels.activity();
        if self.commands.seen_activity == Some(activity) && !self.commands.wake {
            return;
        }
        self.commands.seen_activity = Some(activity);
        self.commands.wake = false;
        for channel in 0..CHANNEL_COUNT {
            self.step_channel(channel);
        }
    }

    /// Routes a finished job to its command.
    pub(super) fn finish_command_job(&mut self, index: usize, result: Result<Vec<u8>, HostError>) {
        self.commands.wake = true;
        let Slot::Active(active) = &mut self.commands.slots[index] else {
            return;
        };
        active.request = None;
        if let Some((drive, image)) = active.absorb(result) {
            self.mount_guest_image(drive, image);
        }
    }

    /// Forgets every command; the caller has reset or replaced the channels.
    pub(super) fn reset_commands(&mut self) {
        self.commands = Commands::default();
    }

    /// Host work is about to be cancelled while channels stay open: hang up
    /// each running command so its client exits instead of waiting.
    pub(super) fn abort_commands(&mut self) {
        for slot in &mut self.commands.slots {
            if let Slot::Active(active) = slot {
                let handle = active.handle;
                let _ = self.channels.hangup(handle);
                *slot = Slot::Passed(handle);
            }
        }
    }

    fn step_channel(&mut self, index: usize) {
        let channel = u8::try_from(index).expect("CHANNEL_COUNT fits a byte");
        let Some(info) = self.channels.info(channel) else {
            return;
        };
        let current = info.handle.filter(|_| info.open);
        if self.commands.slots[index].handle() != current {
            self.drop_command(index);
            if let Some(handle) = current {
                self.commands.slots[index] = Slot::Watching(handle);
            }
        }
        match self.commands.slots[index] {
            Slot::Watching(handle) => self.watch(index, handle),
            Slot::Active(_) => self.advance(index),
            Slot::Idle | Slot::Passed(_) => {}
        }
    }

    /// The guest closed or replaced the session: abandon its work.
    fn drop_command(&mut self, index: usize) {
        if let Slot::Active(active) = std::mem::take(&mut self.commands.slots[index])
            && let Some(id) = active.request
        {
            self.host.cancel_request(id);
        }
    }

    fn watch(&mut self, index: usize, handle: ChannelHandle) {
        let Some(pending) = self.channels.peek(handle) else {
            return;
        };
        let parsed = match line::classify(pending.iter().copied()) {
            Line::Undecided => return,
            Line::NotCommand => {
                self.commands.slots[index] = Slot::Passed(handle);
                return;
            }
            Line::TooLong => {
                let _ = self.channels.receive(handle, line::MAX_LINE_BYTES);
                Err(Failure::new(code::SYNTAX_ERROR, "Command line too long"))
            }
            Line::Complete(len) => {
                let mut bytes = self.channels.receive(handle, len).unwrap_or_default();
                bytes.retain(|&byte| byte != 0);
                bytes.pop();
                parse::parse(&bytes)
            }
        };
        let active = self.start(handle, parsed);
        self.commands.slots[index] = Slot::Active(Box::new(active));
        self.advance(index);
    }

    fn start(&mut self, handle: ChannelHandle, parsed: Result<Command, Failure>) -> Active {
        let mut active = Active::new(handle, Task::Finish);
        match parsed {
            Err(failure) => active.reply_fail(&failure),
            Ok(Command::Text(text)) => active.reply_ok(&text),
            Ok(Command::ServerDir { path }) => {
                active.task = Task::Dir {
                    path,
                    next: 0,
                    page: None,
                };
            }
            Ok(Command::ServerList { path }) => active.task = Task::Open { path, reader: None },
            Ok(Command::DiskInsert { drive, path }) => {
                active.task = Task::Insert {
                    drive,
                    path,
                    image: None,
                };
            }
            Ok(Command::DiskShow { drive }) => self.disk_show(&mut active, drive),
            Ok(Command::DiskEject { drive }) => self.disk_eject(&mut active, drive),
        }
        active
    }

    /// Queues output, then submits the next job or hangs up.
    fn advance(&mut self, index: usize) {
        let Slot::Active(active) = &mut self.commands.slots[index] else {
            return;
        };
        if active.request.is_some() {
            return;
        }
        if !active.output.is_empty() {
            let Ok(accepted) = self
                .channels
                .send(active.handle, active.output.make_contiguous())
            else {
                self.commands.slots[index] = Slot::Idle;
                return;
            };
            active.output.drain(..accepted);
            if !active.output.is_empty() {
                return;
            }
        }
        let Some(job) = active.task.next_job(&self.shares) else {
            let _ = self.channels.hangup(active.handle);
            self.commands.slots[index] = Slot::Passed(active.handle);
            return;
        };
        match self.host.submit(job) {
            Ok(id) => active.request = Some(id),
            Err(SubmitError::Full(_)) => self.commands.wake = true,
            Err(SubmitError::Stopped(_)) => {
                active.reply_fail(&Failure::new(
                    code::SERVER_NOT_READY,
                    "DriveWire host service is not running",
                ));
                self.commands.wake = true;
            }
        }
    }

    fn disk_show(&self, active: &mut Active, drive: Option<usize>) {
        match drive {
            None => active.reply_ok(&reply::disk_list(
                (0..DRIVE_COUNT).filter_map(|drive| Some((drive, self.drive_media(drive)?))),
            )),
            Some(drive) => match self.drive_media(drive) {
                Some(media) => active.reply_ok(&reply::disk_details(drive, media)),
                None => active.reply_fail(&not_loaded(drive)),
            },
        }
    }

    fn disk_eject(&mut self, active: &mut Active, drive: Option<usize>) {
        match drive {
            None => {
                for drive in 0..DRIVE_COUNT {
                    self.eject_for_guest(drive);
                }
                active.reply_ok(b"Ejected all disks.\r\n");
            }
            Some(drive) if self.eject_for_guest(drive) => {
                active.reply_ok(format!("Disk ejected from drive {drive}.\r\n").as_bytes());
            }
            Some(drive) => active.reply_fail(&not_loaded(drive)),
        }
    }
}

fn not_loaded(drive: usize) -> Failure {
    Failure::new(
        code::DRIVE_NOT_LOADED,
        format!("There is no disk in drive {drive}"),
    )
}

#[cfg(test)]
#[path = "command_test.rs"]
pub(super) mod tests;
