//! Bounded asynchronous execution for DriveWire host operations.

#[cfg(test)]
#[path = "host_test.rs"]
mod tests;

use std::collections::HashMap;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use super::share::ShareError;

/// Largest result that a host operation can return to the emulation thread.
pub const MAX_HOST_RESPONSE_BYTES: usize = 4096;

const DEFAULT_QUEUE_CAPACITY: usize = 16;
const MAX_COMPLETIONS_PER_POLL: usize = 8;
const HOST_QUEUE_COUNT: usize = 2;
const ACTIVE_WORKER_COUNT: usize = 1;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// Work executed outside the emulation thread.
pub type HostJob = Box<dyn FnOnce(Cancellation) -> Result<Vec<u8>, HostError> + Send + 'static>;

/// A cooperative cancellation token for a host operation.
#[derive(Clone)]
pub struct Cancellation {
    generation: u64,
    shared: Arc<Shared>,
    request_cancelled: Arc<AtomicBool>,
}

impl Cancellation {
    /// Reports whether the operation's result is no longer wanted.
    pub fn is_cancelled(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
            || self.shared.generation.load(Ordering::Acquire) != self.generation
            || self.request_cancelled.load(Ordering::Acquire)
    }
}

/// A host operation failure with bounded diagnostic data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostError {
    Io(io::ErrorKind),
    /// A share operation failed; see [`super::share`].
    Share(ShareError),
    Cancelled,
    Panicked,
    ResponseTooLarge {
        size: usize,
        max: usize,
    },
}

impl From<io::Error> for HostError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Opaque identity of an accepted request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RequestId {
    session: u64,
    generation: u64,
    sequence: u64,
}

/// Result of a completed request.
#[derive(Debug)]
pub struct HostCompletion {
    pub id: RequestId,
    pub result: Result<Vec<u8>, HostError>,
}

/// Reason that a request was not accepted.
pub enum SubmitError {
    Full(HostJob),
    Stopped(HostJob),
}

impl std::fmt::Debug for SubmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("Full(..)"),
            Self::Stopped(_) => formatter.write_str("Stopped(..)"),
        }
    }
}

/// Executor lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostState {
    Running,
    Suspended,
    Stopped,
}

/// Bounded executor counters for diagnostics and UI reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostDiagnostics {
    pub state: HostState,
    pub pending: usize,
    pub outstanding: usize,
    pub completions: u64,
    pub errors: u64,
    pub cancelled: u64,
    pub stale: u64,
    pub backpressure: u64,
    pub last_error: Option<HostError>,
}

struct Shared {
    generation: AtomicU64,
    stopped: AtomicBool,
    outstanding: AtomicUsize,
}

struct Request {
    id: RequestId,
    cancellation: Cancellation,
    job: HostJob,
}

enum Backend {
    Dormant { capacity: usize },
    Threaded { requests: mpsc::SyncSender<Request> },
    Manual { requests: mpsc::SyncSender<Request> },
    Stopped,
}

/// A bounded host-operation executor owned by one emulated machine.
pub struct HostExecutor {
    session: u64,
    sequence: u64,
    max_outstanding: usize,
    state: HostState,
    shared: Arc<Shared>,
    backend: Backend,
    completions_rx: mpsc::Receiver<HostCompletion>,
    completions_tx: mpsc::SyncSender<HostCompletion>,
    accepted: HashMap<RequestId, Arc<AtomicBool>>,
    diagnostics: HostDiagnostics,
}

impl HostExecutor {
    /// Creates a lazy executor. The worker starts on the first submission.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_QUEUE_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let max_outstanding = capacity
            .saturating_mul(HOST_QUEUE_COUNT)
            .saturating_add(ACTIVE_WORKER_COUNT);
        let (completions_tx, completions_rx) = mpsc::sync_channel(capacity);
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            outstanding: AtomicUsize::new(0),
        });
        Self {
            session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
            sequence: 0,
            max_outstanding,
            state: HostState::Running,
            shared,
            backend: Backend::Dormant { capacity },
            completions_rx,
            completions_tx,
            accepted: HashMap::with_capacity(max_outstanding),
            diagnostics: HostDiagnostics {
                state: HostState::Running,
                pending: 0,
                outstanding: 0,
                completions: 0,
                errors: 0,
                cancelled: 0,
                stale: 0,
                backpressure: 0,
                last_error: None,
            },
        }
    }

    /// Creates a deterministic executor without a worker thread.
    pub fn manual(capacity: usize) -> (Self, ManualHost) {
        let mut executor = Self::with_capacity(capacity);
        let (requests, requests_rx) = mpsc::sync_channel(capacity.max(1));
        executor.backend = Backend::Manual { requests };
        let host = ManualHost {
            requests: requests_rx,
            completions: executor.completions_tx.clone(),
        };
        (executor, host)
    }

    /// Attempts to enqueue work without waiting for queue capacity.
    pub fn submit(&mut self, job: HostJob) -> Result<RequestId, SubmitError> {
        if self.state != HostState::Running {
            return Err(SubmitError::Stopped(job));
        }
        if self.shared.outstanding.load(Ordering::Acquire) >= self.max_outstanding {
            self.diagnostics.backpressure += 1;
            return Err(SubmitError::Full(job));
        }
        if let Err(error) = self.ensure_worker() {
            self.diagnostics.errors += 1;
            self.diagnostics.last_error = Some(HostError::Io(error));
            self.shared.stopped.store(true, Ordering::Release);
            self.backend = Backend::Stopped;
            self.set_state(HostState::Stopped);
            return Err(SubmitError::Stopped(job));
        }
        let id = self.next_request_id();
        let request_cancelled = Arc::new(AtomicBool::new(false));
        let request = Request {
            id,
            cancellation: Cancellation {
                generation: id.generation,
                shared: Arc::clone(&self.shared),
                request_cancelled: Arc::clone(&request_cancelled),
            },
            job,
        };
        let sender = match &self.backend {
            Backend::Threaded { requests } | Backend::Manual { requests } => requests,
            Backend::Dormant { .. } | Backend::Stopped => unreachable!(),
        };
        match sender.try_send(request) {
            Ok(()) => {
                self.diagnostics.pending += 1;
                self.shared.outstanding.fetch_add(1, Ordering::AcqRel);
                self.accepted.insert(id, request_cancelled);
                Ok(id)
            }
            Err(mpsc::TrySendError::Full(request)) => {
                self.diagnostics.backpressure += 1;
                Err(SubmitError::Full(request.job))
            }
            Err(mpsc::TrySendError::Disconnected(request)) => {
                self.stop();
                Err(SubmitError::Stopped(request.job))
            }
        }
    }

    /// Returns one current completion without waiting.
    pub fn poll(&mut self) -> Option<HostCompletion> {
        for _ in 0..MAX_COMPLETIONS_PER_POLL {
            let completion = match self.completions_rx.try_recv() {
                Ok(completion) => completion,
                Err(_) => break,
            };
            let Some(request_cancelled) = self.accepted.remove(&completion.id) else {
                self.diagnostics.stale += 1;
                continue;
            };
            self.shared.outstanding.fetch_sub(1, Ordering::AcqRel);
            if request_cancelled.load(Ordering::Acquire)
                || completion.id.generation != self.current_generation()
            {
                self.diagnostics.stale += 1;
                continue;
            }
            self.record_completion(&completion);
            return Some(completion);
        }
        None
    }

    /// Invalidates all accepted work and results from the current generation.
    pub fn cancel(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.diagnostics.cancelled += self.diagnostics.pending as u64;
        self.diagnostics.pending = 0;
    }

    /// Invalidates one accepted request while preserving unrelated work.
    pub fn cancel_request(&mut self, id: RequestId) -> bool {
        if id.generation != self.current_generation() {
            return false;
        }
        let Some(cancelled) = self.accepted.get(&id) else {
            return false;
        };
        if cancelled.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.diagnostics.pending = self.diagnostics.pending.saturating_sub(1);
        self.diagnostics.cancelled += 1;
        true
    }

    /// Invalidates current work and rejects submissions until resumed.
    pub fn suspend(&mut self) {
        if self.state == HostState::Running {
            self.cancel();
            self.set_state(HostState::Suspended);
        }
    }

    /// Accepts submissions after a suspension.
    pub fn resume(&mut self) {
        if self.state == HostState::Suspended {
            self.set_state(HostState::Running);
        }
    }

    /// Permanently rejects submissions and invalidates current work.
    pub fn stop(&mut self) {
        if self.state != HostState::Stopped {
            self.cancel();
            self.shared.stopped.store(true, Ordering::Release);
            self.backend = Backend::Stopped;
            self.set_state(HostState::Stopped);
        }
    }

    /// Reports whether no physical host work or undelivered result remains.
    pub fn quiescent(&self) -> bool {
        self.shared.outstanding.load(Ordering::Acquire) == 0
    }

    /// Reports whether no physical host work or undelivered result remains.
    pub fn is_idle(&self) -> bool {
        self.quiescent()
    }

    /// Returns a snapshot of executor diagnostics.
    pub fn diagnostics(&self) -> HostDiagnostics {
        let mut diagnostics = self.diagnostics;
        diagnostics.outstanding = self.shared.outstanding.load(Ordering::Acquire);
        diagnostics
    }

    fn ensure_worker(&mut self) -> Result<(), io::ErrorKind> {
        let capacity = match self.backend {
            Backend::Dormant { capacity } => capacity,
            _ => return Ok(()),
        };
        let (requests, requests_rx) = mpsc::sync_channel(capacity);
        let completions = self.completions_tx.clone();
        thread::Builder::new()
            .name("cocovm-drivewire-host".into())
            .spawn(move || worker_loop(requests_rx, completions))
            .map_err(|error| error.kind())?;
        self.backend = Backend::Threaded { requests };
        Ok(())
    }

    fn next_request_id(&mut self) -> RequestId {
        let id = RequestId {
            session: self.session,
            generation: self.current_generation(),
            sequence: self.sequence,
        };
        self.sequence = self.sequence.wrapping_add(1);
        id
    }

    fn current_generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    fn record_completion(&mut self, completion: &HostCompletion) {
        self.diagnostics.pending = self.diagnostics.pending.saturating_sub(1);
        self.diagnostics.completions += 1;
        if let Err(error) = completion.result {
            self.diagnostics.errors += 1;
            self.diagnostics.last_error = Some(error);
        }
    }

    fn set_state(&mut self, state: HostState) {
        self.state = state;
        self.diagnostics.state = state;
    }
}

impl Default for HostExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HostExecutor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_loop(requests: mpsc::Receiver<Request>, completions: mpsc::SyncSender<HostCompletion>) {
    while let Ok(request) = requests.recv() {
        let completion = run_request(request);
        if completions.send(completion).is_err() {
            break;
        }
    }
}

fn run_request(request: Request) -> HostCompletion {
    let Request {
        id,
        cancellation,
        job,
    } = request;
    let result = if cancellation.is_cancelled() {
        Err(HostError::Cancelled)
    } else {
        match panic::catch_unwind(AssertUnwindSafe(|| job(cancellation.clone()))) {
            Ok(result) => normalize_result(result, &cancellation),
            Err(_) => Err(HostError::Panicked),
        }
    };
    HostCompletion { id, result }
}

fn normalize_result(
    result: Result<Vec<u8>, HostError>,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, HostError> {
    if cancellation.is_cancelled() {
        return Err(HostError::Cancelled);
    }
    match result {
        Ok(bytes) if bytes.len() > MAX_HOST_RESPONSE_BYTES => Err(HostError::ResponseTooLarge {
            size: bytes.len(),
            max: MAX_HOST_RESPONSE_BYTES,
        }),
        result => result,
    }
}

/// Manual side of a deterministic host executor.
pub struct ManualHost {
    requests: mpsc::Receiver<Request>,
    completions: mpsc::SyncSender<HostCompletion>,
}

impl ManualHost {
    /// Takes one queued request without waiting.
    pub fn take_request(&self) -> Option<ManualRequest> {
        self.requests.try_recv().ok().map(ManualRequest)
    }

    /// Publishes a result without waiting for completion-queue capacity.
    pub fn complete(&self, completion: HostCompletion) -> Result<(), HostCompletion> {
        match self.completions.try_send(completion) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(completion))
            | Err(mpsc::TrySendError::Disconnected(completion)) => Err(completion),
        }
    }
}

/// A request held by a [`ManualHost`].
pub struct ManualRequest(Request);

impl ManualRequest {
    /// Returns the request identity.
    pub fn id(&self) -> RequestId {
        self.0.id
    }

    /// Runs this request synchronously for deterministic tests.
    pub fn run(self) -> HostCompletion {
        run_request(self.0)
    }
}
