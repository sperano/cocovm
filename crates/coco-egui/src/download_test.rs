use std::io::Read;
use std::sync::Arc;

use super::*;

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
