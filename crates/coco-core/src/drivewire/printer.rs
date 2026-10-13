//! DriveWire printing: `OP_PRINT` collects one byte into the open job and
//! `OP_PRINTFLUSH` ends it (DriveWire specification `a795310`, "Printing";
//! neither has a reply). NitrOS-9's `scdwp` driver behind `/P` sends
//! `OP_PRINT` and the raw byte for every character and `OP_PRINTFLUSH` when
//! the last path to `/P` closes (`level1/modules/scdwp.asm` at `0c9940f`);
//! HDB-DOS never prints over DriveWire.
//!
//! The open job is guest state and travels in snapshots. Finished jobs are
//! host work and never do: a running session delivers what it finished, so
//! restoring a snapshot cannot deliver a job twice. Where a job goes is
//! [`PrintOutput`], installed by the frontend; until then, and with
//! [`PrintOutput::Off`], finished jobs are discarded and counted.
//!
//! Policies, each counted in [`PrintDiagnostics`]:
//!
//! - **Empty jobs**: a flush with nothing collected produces no output.
//! - **Large jobs**: memory stays bounded by ending a part every
//!   [`PRINT_JOB_PART_BYTES`]; the job continues in the next part, and each
//!   part is delivered on its own (numbered in its file name).
//! - **Queue**: at most [`PRINT_QUEUE_JOBS`] finished jobs wait for a slow
//!   or failing folder. The wire has no flow control for printing, so a job
//!   finished while the queue is full is dropped.
//! - **Output errors**: a failed folder write keeps its job queued. Queued
//!   jobs are retried when the next job finishes or the output changes.
//!   Switching output re-routes queued jobs; switching to off discards them.
//! - **Resets**: `DWINIT` and protocol resets keep the open job, as the
//!   DriveWire 4 Java server does (its `DWVPrinter` outlives them). A machine
//!   reset or stop ends the open job as an incomplete one, since no guest
//!   is left to flush it. Stopping hands queued jobs to the writer thread
//!   for a last attempt.
//! - **Suspend**: the open job belongs to the suspend state, so a stop after
//!   suspending leaves it there instead of ending it.

use std::collections::VecDeque;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{DWServer, DWTime};

mod output;
use output::{FolderWriter, WriteResult};
pub use output::{PrintFormat, PrintOutput};

/// A job is delivered in parts of at most this many bytes, bounding the
/// open job (and the snapshot that carries it).
pub const PRINT_JOB_PART_BYTES: usize = 1024 * 1024;

/// Finished jobs that can wait for folder output at once, including the
/// ones being written.
pub const PRINT_QUEUE_JOBS: usize = 16;

/// Why a delivered job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobEnd {
    /// The guest flushed it.
    Flushed,
    /// It reached [`PRINT_JOB_PART_BYTES`]; the job continues in the next
    /// part.
    Split,
    /// A machine reset or stop ended it before the guest flushed it.
    Unterminated,
}

/// A finished job, or one part of a large one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintJob {
    /// Job number in this VM's session, from 1.
    pub number: u64,
    /// Part number within the job, from 1.
    pub part: u32,
    pub end: JobEnd,
    /// Host time when the part ended.
    pub stamp: DWTime,
    pub bytes: Vec<u8>,
}

/// Counters and state for the settings pane and status bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrintDiagnostics {
    /// Bytes collected for the open job's current part.
    pub open_bytes: usize,
    /// Finished jobs not yet written, including ones that failed.
    pub waiting: usize,
    /// Jobs (and parts) delivered to a folder or the printer port.
    pub printed: u64,
    /// Jobs discarded because output was off.
    pub discarded: u64,
    /// Jobs dropped because the queue was full.
    pub dropped: u64,
    /// Flushes with nothing to print.
    pub empty: u64,
    /// Jobs ended by a machine reset or stop.
    pub incomplete: u64,
    /// The last file written.
    pub last_file: Option<PathBuf>,
    /// The last output failure, kept until a later write succeeds.
    pub last_error: Option<String>,
}

/// One VM's print spool. Only the open job and the job counter are saved.
#[derive(Default, Serialize, Deserialize)]
pub(super) struct PrintSpool {
    /// The open job's current part.
    open: Vec<u8>,
    /// Jobs ended so far; the open job is number `ended + 1`.
    ended: u64,
    /// Parts of the open job already delivered.
    parts: u32,
    #[serde(skip)]
    output: PrintOutput,
    /// Folder jobs not yet handed to the writer.
    #[serde(skip)]
    queue: VecDeque<PrintJob>,
    /// Jobs handed to the writer without a result yet.
    #[serde(skip)]
    writing: usize,
    /// A write failed: wait for a new job or output before retrying.
    #[serde(skip)]
    blocked: bool,
    #[serde(skip)]
    writer: Option<FolderWriter>,
    /// Printer-port jobs for the bus to deliver.
    #[serde(skip)]
    port: VecDeque<PrintJob>,
    #[serde(skip)]
    suspended: bool,
    #[serde(skip)]
    stats: PrintDiagnostics,
}

impl PrintSpool {
    /// Adds `byte` to the open part; true when the part is full.
    pub(super) fn collect(&mut self, byte: u8) -> bool {
        self.open.push(byte);
        self.open.len() >= PRINT_JOB_PART_BYTES
    }

    /// Ends the open part. An empty flush ends the job without output.
    pub(super) fn end(&mut self, end: JobEnd, stamp: DWTime) {
        if self.open.is_empty() {
            if end == JobEnd::Flushed && self.parts == 0 {
                self.stats.empty += 1;
            }
            if end != JobEnd::Split && self.parts > 0 {
                self.ended += 1;
                self.parts = 0;
            }
            return;
        }
        self.parts += 1;
        let job = PrintJob {
            number: self.ended + 1,
            part: self.parts,
            end,
            stamp,
            bytes: std::mem::take(&mut self.open),
        };
        if end != JobEnd::Split {
            self.ended += 1;
            self.parts = 0;
        }
        if end == JobEnd::Unterminated {
            self.stats.incomplete += 1;
        }
        self.deliver(job);
    }

    fn deliver(&mut self, job: PrintJob) {
        match self.output {
            PrintOutput::Off => self.stats.discarded += 1,
            PrintOutput::PrinterPort => self.port.push_back(job),
            PrintOutput::Folder { .. } => {
                if self.queue.len() + self.writing >= PRINT_QUEUE_JOBS {
                    self.stats.dropped += 1;
                    self.stats.last_error = Some(format!(
                        "job {} dropped: {PRINT_QUEUE_JOBS} jobs are already waiting",
                        job.number
                    ));
                    return;
                }
                self.queue.push_back(job);
                self.blocked = false;
                self.pump();
            }
        }
    }

    pub(super) fn set_output(&mut self, output: PrintOutput) {
        self.output = output;
        self.blocked = false;
        for job in std::mem::take(&mut self.queue) {
            self.deliver(job);
        }
    }

    pub(super) fn output(&self) -> &PrintOutput {
        &self.output
    }

    /// Hands queued folder jobs to the writer and collects its results.
    pub(super) fn poll(&mut self) {
        if let Some(writer) = &self.writer {
            while let Some(result) = writer.try_result() {
                self.writing -= 1;
                match result {
                    WriteResult::Written(path) => {
                        self.stats.printed += 1;
                        self.stats.last_file = Some(path);
                        self.stats.last_error = None;
                    }
                    WriteResult::Failed { job, error } => {
                        self.stats.last_error = Some(error);
                        self.blocked = true;
                        self.queue.push_front(job);
                    }
                }
            }
        }
        self.pump();
    }

    fn pump(&mut self) {
        let PrintOutput::Folder { dir, format } = &self.output else {
            return;
        };
        while !self.blocked
            && let Some(job) = self.queue.pop_front()
        {
            if self.writer.is_none() {
                match FolderWriter::spawn() {
                    Ok(writer) => self.writer = Some(writer),
                    Err(error) => {
                        self.stats.last_error = Some(format!("printer thread: {error}"));
                        self.blocked = true;
                        self.queue.push_front(job);
                        return;
                    }
                }
            }
            let writer = self.writer.as_ref().expect("spawned above");
            match writer.send(job, dir.clone(), *format) {
                Ok(()) => self.writing += 1,
                Err(job) => {
                    self.writer = None;
                    self.stats.last_error = Some("printer thread stopped".into());
                    self.blocked = true;
                    self.queue.push_front(job);
                }
            }
        }
    }

    pub(super) fn take_port_job(&mut self) -> Option<PrintJob> {
        let job = self.port.pop_front()?;
        self.stats.printed += 1;
        Some(job)
    }

    pub(super) fn has_port_job(&self) -> bool {
        !self.port.is_empty()
    }

    pub(super) fn suspend(&mut self) {
        self.suspended = true;
    }

    pub(super) fn resume(&mut self) {
        self.suspended = false;
    }

    /// Ends the open job unless it belongs to a suspend state, hands every
    /// queued job to the writer for a last attempt, and lets the writer
    /// finish on its own.
    pub(super) fn stop(&mut self, stamp: DWTime) {
        if self.suspended {
            self.open.clear();
            self.parts = 0;
        } else {
            self.end(JobEnd::Unterminated, stamp);
        }
        self.blocked = false;
        self.pump();
        if !self.queue.is_empty() {
            eprintln!(
                "DriveWire printer: {} jobs not written at stop: {}",
                self.queue.len(),
                self.stats.last_error.as_deref().unwrap_or("no output")
            );
            self.queue.clear();
        }
        self.writer = None;
        self.writing = 0;
        self.suspended = false;
    }

    pub(super) fn diagnostics(&self) -> PrintDiagnostics {
        PrintDiagnostics {
            open_bytes: self.open.len(),
            waiting: self.queue.len() + self.writing,
            ..self.stats.clone()
        }
    }
}

impl DWServer {
    /// Chooses where finished print jobs go; see [`PrintOutput`]. Queued
    /// jobs follow the new choice.
    pub fn set_print_output(&mut self, output: PrintOutput) {
        self.printer.set_output(output);
    }

    pub fn print_output(&self) -> &PrintOutput {
        self.printer.output()
    }

    pub fn print_diagnostics(&self) -> PrintDiagnostics {
        self.printer.diagnostics()
    }

    /// Takes the next job for [`PrintOutput::PrinterPort`]; the bus feeds it
    /// to the bit-banger's printer.
    pub fn take_port_print_job(&mut self) -> Option<PrintJob> {
        self.printer.take_port_job()
    }

    pub fn has_port_print_job(&self) -> bool {
        self.printer.has_port_job()
    }

    /// A machine reset: [`Self::reset_session`], and the open print job
    /// ends as incomplete, since the guest that started it is gone.
    pub fn machine_reset(&mut self) {
        self.reset_session();
        let stamp = (self.clock)();
        self.printer.end(JobEnd::Unterminated, stamp);
    }

    /// [`super::opcode::PRINT`]'s data byte.
    pub(super) fn print_byte(&mut self, byte: u8) {
        if self.printer.collect(byte) {
            let stamp = (self.clock)();
            self.printer.end(JobEnd::Split, stamp);
        }
    }

    /// [`super::opcode::PRINTFLUSH`].
    pub(super) fn print_flush(&mut self) {
        let stamp = (self.clock)();
        self.printer.end(JobEnd::Flushed, stamp);
    }
}

#[cfg(test)]
#[path = "printer_test.rs"]
mod tests;
