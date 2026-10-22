//! HTTP downloads with byte-level progress, shared by the first-run asset
//! download (`startup::download_and_unpack_assets`) and the NitrOS-9 EOU
//! download (`nitros9::download_eou`). Each download streams through a
//! [`ProgressReader`] that bumps a [`Progress`] the UI thread reads, over a
//! [`StallGuard`] that fails it once the body stops arriving.

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// How long [`open`] waits to connect, so a dead host fails instead of
/// leaving a progress bar at zero.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long [`open`] waits for the response headers once connected.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a body may go without a single byte before its download fails
/// ([`StallGuard`]). The body as a whole has no limit: ureq only bounds its
/// total time, which would cut off a slow download that is still making
/// progress.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Largest chunk [`StallGuard`]'s reading thread hands over at a time.
const STALL_GUARD_CHUNK_BYTES: usize = 64 * 1024;

/// Chunks [`StallGuard`]'s reading thread may read ahead of its consumer.
const STALL_GUARD_QUEUE_CHUNKS: usize = 4;

/// Bytes a download has received out of how many it expects. Written by the
/// download thread, read by the UI; the two counters are independent, so
/// relaxed ordering is enough.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    received: AtomicU64,
    /// The response's `Content-Length`; 0 while unknown.
    total: AtomicU64,
}

impl Progress {
    /// Bytes received so far.
    pub(crate) fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }

    /// Bytes expected in all, when the server said.
    pub(crate) fn total(&self) -> Option<u64> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then_some(total)
    }

    /// [`Self::received`] as a share of [`Self::total`], in `0.0..=1.0`;
    /// `None` while the total is unknown.
    pub(crate) fn fraction(&self) -> Option<f32> {
        self.total()
            .map(|total| (self.received() as f64 / total as f64).min(1.0) as f32)
    }

    /// Expect `total` bytes in all; `None` (stored as 0) when unknown.
    pub(crate) fn set_total(&self, total: Option<u64>) {
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    /// Count `bytes` more as received.
    pub(crate) fn add(&self, bytes: u64) {
        self.received.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// A reader that adds every byte read from `inner` to a [`Progress`].
pub(crate) struct ProgressReader<R> {
    inner: R,
    progress: Arc<Progress>,
}

impl<R> ProgressReader<R> {
    pub(crate) fn new(inner: R, progress: Arc<Progress>) -> Self {
        Self { inner, progress }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.progress.add(n as u64);
        Ok(n)
    }
}

/// A reader that fails with [`io::ErrorKind::TimedOut`] once `inner` has
/// produced nothing for its timeout. ureq has no such idle limit, and a
/// blocked socket read can't be interrupted, so `inner` is read on a thread
/// of its own that hands chunks over a bounded channel. On a stall, that
/// thread is abandoned: it exits on its next read, once the socket gives up.
pub(crate) struct StallGuard {
    chunks: mpsc::Receiver<io::Result<Vec<u8>>>,
    timeout: Duration,
    /// The chunk being handed out, and how much of it has been.
    chunk: Vec<u8>,
    consumed: usize,
    /// Whether `inner` has reached its end.
    finished: bool,
}

impl StallGuard {
    /// Read `inner` on a new thread, failing after `timeout` without data.
    pub(crate) fn new(inner: impl Read + Send + 'static, timeout: Duration) -> Self {
        let (tx, chunks) = mpsc::sync_channel(STALL_GUARD_QUEUE_CHUNKS);
        std::thread::spawn(move || pump_chunks(inner, &tx));
        Self {
            chunks,
            timeout,
            chunk: Vec::new(),
            consumed: 0,
            finished: false,
        }
    }

    /// Wait for the next chunk into [`Self::chunk`]; `false` at the end.
    fn next_chunk(&mut self) -> io::Result<bool> {
        let chunk = match self.chunks.recv_timeout(self.timeout) {
            Ok(chunk) => chunk?,
            Err(RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no data received for {} s", self.timeout.as_secs()),
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("download reader exited unexpectedly"));
            }
        };
        self.finished = chunk.is_empty();
        self.chunk = chunk;
        self.consumed = 0;
        Ok(!self.finished)
    }
}

impl Read for StallGuard {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.consumed == self.chunk.len() && (self.finished || !self.next_chunk()?) {
            return Ok(0);
        }
        let rest = &self.chunk[self.consumed..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.consumed += n;
        Ok(n)
    }
}

/// [`StallGuard`]'s reading thread: send `inner`'s chunks, ending with an
/// empty one at its end or with its error, until the guard is dropped.
fn pump_chunks(mut inner: impl Read, tx: &mpsc::SyncSender<io::Result<Vec<u8>>>) {
    loop {
        let mut chunk = vec![0; STALL_GUARD_CHUNK_BYTES];
        let read = match inner.read(&mut chunk) {
            Ok(n) => {
                chunk.truncate(n);
                Ok(chunk)
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => Err(e),
        };
        let last = !matches!(&read, Ok(chunk) if !chunk.is_empty());
        if tx.send(read).is_err() || last {
            return;
        }
    }
}

/// GET `url` and return its body as a stream that reports into `progress`,
/// with the response's length as its total, and fails once the body stalls
/// for [`STALL_TIMEOUT`]. `progress` should be fresh: it counts on from
/// whatever it already holds.
pub(crate) fn open(
    url: &str,
    progress: Arc<Progress>,
) -> Result<ProgressReader<StallGuard>, ureq::Error> {
    let body = ureq::get(url)
        .config()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .build()
        .call()?
        .into_body();
    progress.set_total(body.content_length());
    let body = StallGuard::new(body.into_reader(), STALL_TIMEOUT);
    Ok(ProgressReader::new(body, progress))
}

#[cfg(test)]
#[path = "download_test.rs"]
mod tests;
