//! HTTP downloads with byte-level progress, shared by the first-run asset
//! download (`startup::download_and_unpack_assets`) and the NitrOS-9 EOU
//! download (`nitros9::download_eou`). Each download streams through a
//! [`ProgressReader`] that bumps a [`Progress`] the UI thread reads.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
        Some(self.total.load(Ordering::Relaxed)).filter(|&total| total > 0)
    }

    /// [`Self::received`] as a share of [`Self::total`], in `0.0..=1.0`;
    /// `None` while the total is unknown.
    pub(crate) fn fraction(&self) -> Option<f32> {
        self.total()
            .map(|total| (self.received() as f64 / total as f64).min(1.0) as f32)
    }

    pub(crate) fn set_total(&self, total: Option<u64>) {
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

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

/// GET `url` and return its body as a stream that reports into `progress`,
/// with the response's length as its total. `progress` should be fresh:
/// it counts on from whatever it already holds.
pub(crate) fn open(
    url: &str,
    progress: Arc<Progress>,
) -> Result<ProgressReader<ureq::BodyReader<'static>>, ureq::Error> {
    let body = ureq::get(url).call()?.into_body();
    progress.set_total(body.content_length());
    Ok(ProgressReader::new(body.into_reader(), progress))
}

#[cfg(test)]
#[path = "download_test.rs"]
mod tests;
