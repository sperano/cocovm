//! Host work and session boundaries. Host handles and jobs never enter snapshots.

use super::host::{
    HostCompletion, HostDiagnostics, HostError, HostExecutor, HostJob, RequestId, SubmitError,
};
use super::protocol::State;
use super::{DWServer, SECTOR_SIZE, error};

const SERVICE_COMPLETION_CAPACITY: usize = 16;

pub(super) struct PendingHost {
    drive: usize,
    job: Option<HostJob>,
    id: Option<RequestId>,
}

impl DWServer {
    /// Builds a server with an injected executor, including a deterministic host.
    pub fn with_host_executor(host: HostExecutor) -> Self {
        Self {
            host,
            ..Self::new()
        }
    }

    /// Submits a bounded service job without waiting. Polling routes its result
    /// to [`Self::take_host_completion`]; lifecycle changes invalidate it.
    pub fn submit_host_service(&mut self, job: HostJob) -> Result<RequestId, SubmitError> {
        self.host.submit(job)
    }

    pub fn host_diagnostics(&self) -> HostDiagnostics {
        let mut diagnostics = self.host.diagnostics();
        diagnostics.pending += self.service_completions.len();
        diagnostics.pending +=
            usize::from(self.pending_host.as_ref().is_some_and(|p| p.job.is_some()));
        diagnostics
    }

    /// Includes cancelled work whose host side effects have not finished yet.
    pub fn host_is_idle(&self) -> bool {
        self.pending_host.is_none() && self.service_completions.is_empty() && self.host.is_idle()
    }

    /// Takes a completion submitted by a service other than the disk parser.
    pub fn take_host_completion(&mut self) -> Option<HostCompletion> {
        self.service_completions.pop_front()
    }

    pub(super) fn begin_host_request(&mut self, drive: usize, job: HostJob) {
        self.pending_host = Some(PendingHost {
            drive,
            job: Some(job),
            id: None,
        });
        self.poll_host();
    }

    /// Makes bounded progress without waiting for a worker or a host handle,
    /// including the `dw` command service on the virtual channels.
    pub fn poll_host(&mut self) {
        if self.service_completions.len() < SERVICE_COMPLETION_CAPACITY {
            self.submit_pending();
            if let Some(completion) = self.host.poll() {
                self.route_completion(completion);
            }
        }
        self.step_commands();
    }

    fn route_completion(&mut self, completion: HostCompletion) {
        let matches = self
            .pending_host
            .as_ref()
            .is_some_and(|pending| pending.id == Some(completion.id));
        if matches {
            self.finish_host_request(completion.result);
        } else if let Some(channel) = self.commands.owner(completion.id) {
            self.finish_command_job(channel, completion.result);
        } else {
            self.service_completions.push_back(completion);
        }
    }

    fn submit_pending(&mut self) {
        let Some(pending) = self.pending_host.as_mut() else {
            return;
        };
        let Some(job) = pending.job.take() else {
            return;
        };
        match self.host.submit(job) {
            Ok(id) => pending.id = Some(id),
            Err(SubmitError::Full(job)) => pending.job = Some(job),
            Err(SubmitError::Stopped(_)) => {
                self.finish_host_request(Err(HostError::Cancelled));
            }
        }
    }

    fn finish_host_request(&mut self, result: Result<Vec<u8>, HostError>) {
        let Some(pending) = self.pending_host.take() else {
            return;
        };
        // Host latency is not an incomplete guest payload timeout. Start timing
        // again at the first checksum byte after an extended-read response.
        self.last_byte_cycle = None;
        match std::mem::replace(&mut self.state, State::Idle) {
            State::AwaitHostRead { ex } => {
                let sector: Result<[u8; SECTOR_SIZE], u8> = result
                    .map_err(|_| error::READ)
                    .and_then(|bytes| bytes.try_into().map_err(|_| error::READ));
                self.finish_read(ex, pending.drive, sector);
            }
            State::AwaitHostWrite => {
                let status = if result.is_ok() {
                    error::OK
                } else {
                    error::WRITE
                };
                self.finish_write(pending.drive, status);
            }
            state => self.state = state,
        }
    }

    pub(super) fn cancel_host_request(&mut self) {
        if let Some(id) = self.pending_host.take().and_then(|pending| pending.id) {
            self.host.cancel_request(id);
        }
    }

    pub(super) fn invalidate_drive_request(&mut self, drive: usize) {
        if self
            .pending_host
            .as_ref()
            .is_some_and(|pending| pending.drive == drive)
        {
            // Disk operations are request/reply transactions. Reject the old
            // operation before changing media, while retaining reply framing.
            if let Some(id) = self.pending_host.as_ref().and_then(|pending| pending.id) {
                self.host.cancel_request(id);
            }
            self.abort_host_transfer();
        }
    }

    /// Out-of-band machine reset cancels transactions and closes virtual
    /// channels, preserving mounted media. The share session starts over:
    /// handles close, the directory returns to the top, and running `dw`
    /// commands end. `DWINIT` and protocol resets start the same way, since
    /// each begins a new guest driver session.
    pub fn reset_session(&mut self) {
        self.host.cancel();
        self.pending_host = None;
        self.service_completions.clear();
        self.channels.reset();
        self.reset_commands();
        self.shares.reset();
        self.state = State::Idle;
        self.reply.clear();
        self.last_byte_cycle = None;
    }

    pub(super) fn protocol_reset(&mut self) {
        self.reset_session();
        self.sectors_read = 0;
        self.sectors_written = 0;
        self.drive_ops.fill(0);
        self.vserial_ops = 0;
        self.unknown_opcodes = 0;
        self.channels.clear_counters();
    }

    /// Suspend closes host services: running `dw` commands hang up. Resume
    /// starts a fresh host generation.
    pub fn suspend_host(&mut self) {
        self.abort_commands();
        self.host.suspend();
        self.service_completions.clear();
        self.shares.reset();
        self.abort_host_transfer();
    }

    pub fn resume_host(&mut self) {
        self.host.resume();
    }

    /// Cancels queued work without joining a possibly blocked worker.
    pub fn stop_host(&mut self) {
        self.host.stop();
        self.pending_host = None;
        self.service_completions.clear();
        self.channels.reset();
        self.reset_commands();
        self.shares.reset();
        self.state = State::Idle;
        self.reply.clear();
        self.last_byte_cycle = None;
    }

    /// Reattached media survives restore; external requests are never
    /// replayed, open virtual channels hang up once drained (ending any `dw`
    /// command), and the share session starts over.
    pub fn after_restore(&mut self) {
        self.host.cancel();
        self.service_completions.clear();
        self.channels.after_restore();
        self.reset_commands();
        self.shares.reset();
        self.abort_host_transfer();
    }

    fn abort_host_transfer(&mut self) {
        self.pending_host = None;
        match std::mem::replace(&mut self.state, State::Idle) {
            State::AwaitHostRead { ex } => {
                self.last_byte_cycle = None;
                self.finish_read(ex, 0, Err(error::READ));
            }
            State::AwaitHostWrite => {
                self.last_byte_cycle = None;
                self.reply.push_back(error::WRITE);
            }
            state => self.state = state,
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_test.rs"]
mod tests;
