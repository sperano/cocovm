//! Newline-delimited JSON: one message per line, in both directions.

use std::io::{self, BufRead, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Longest line either side accepts; a screenshot reply is the biggest
/// legitimate message (a 640×240 PNG, base64) and sits far below this.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Read one message. `Ok(None)` at a clean end of stream; a line that is not
/// valid JSON for `T` is an `InvalidData` error.
pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    let mut limited = Read::take(reader, MAX_LINE_BYTES as u64);
    if limited.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') && line.len() >= MAX_LINE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write one message followed by a newline and flush.
pub fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}
