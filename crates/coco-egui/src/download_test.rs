use std::io::Read;
use std::sync::Arc;
use std::time::Instant;

use super::*;

/// A stall timeout short enough to keep the stall test quick.
const TEST_STALL_TIMEOUT: Duration = Duration::from_millis(50);

/// A reader that blocks until its sender is dropped, like a socket whose
/// server stopped sending.
struct Stalled(mpsc::Receiver<()>);

impl Read for Stalled {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        let _ = self.0.recv();
        Ok(0)
    }
}

/// A reader that fails on its first read.
struct Broken;

impl Read for Broken {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("connection reset"))
    }
}

/// Every byte read through the wrapper is counted, and the fraction follows
/// once a total is known.
#[test]
fn progress_reader_counts_bytes_read() {
    const BODY: &[u8] = b"0123456789";
    let progress = Arc::new(Progress::default());
    progress.set_total(Some(BODY.len() as u64 * 2));
    let mut reader = ProgressReader::new(BODY, Arc::clone(&progress));

    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();

    assert_eq!(out, BODY);
    assert_eq!(progress.received(), BODY.len() as u64);
    assert_eq!(progress.fraction(), Some(0.5));
}

/// Without a `Content-Length` there is nothing to measure against.
#[test]
fn unknown_total_has_no_fraction() {
    let progress = Progress::default();
    progress.add(42);
    assert_eq!(progress.total(), None);
    assert_eq!(progress.fraction(), None);
}

/// A body longer than its announced length never reads past 100%.
#[test]
fn fraction_is_capped_at_one() {
    let progress = Progress::default();
    progress.set_total(Some(10));
    progress.add(15);
    assert_eq!(progress.fraction(), Some(1.0));
}

/// The guard hands over a body longer than one chunk byte for byte.
#[test]
fn stall_guard_passes_the_body_through() {
    let body: Vec<u8> = (0..STALL_GUARD_CHUNK_BYTES * 3 + 7)
        .map(|i| (i % 251) as u8)
        .collect();
    let mut reader = StallGuard::new(io::Cursor::new(body.clone()), STALL_TIMEOUT);

    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();

    assert_eq!(out, body);
    assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0, "stays at its end");
}

/// A body that stops arriving fails once the timeout passes, instead of
/// blocking forever.
#[test]
fn stall_guard_times_out_a_stalled_body() {
    let (_keep_stalled, rx) = mpsc::channel();
    let mut reader = StallGuard::new(Stalled(rx), TEST_STALL_TIMEOUT);

    let started = Instant::now();
    let error = reader.read(&mut [0; 1]).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() >= TEST_STALL_TIMEOUT);
}

/// The inner reader's error reaches the consumer.
#[test]
fn stall_guard_passes_errors_through() {
    let mut reader = StallGuard::new(Broken, STALL_TIMEOUT);
    let error = reader.read(&mut [0; 1]).unwrap_err();
    assert_eq!(error.to_string(), "connection reset");
}
