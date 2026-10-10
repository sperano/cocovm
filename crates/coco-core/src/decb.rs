//! Parser for canonical Disk Extended Color BASIC (DECB) `LOADM` binaries.

use std::fmt;

/// A data block from a canonical DECB binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecbSegment {
    /// Logical address of the first byte. Later bytes wrap past `$FFFF`.
    pub address: u16,
    pub bytes: Vec<u8>,
}

/// A fully parsed canonical DECB binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecbBinary {
    pub segments: Vec<DecbSegment>,
    pub exec_address: u16,
}

/// A malformed or incomplete canonical DECB binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecbError(String);

impl fmt::Display for DecbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DecbError {}

/// Data-block marker emitted by Disk Extended Color BASIC 1.1 `SAVEM`.
const DATA_MARKER: u8 = 0x00;
/// Execution-address trailer marker emitted by Disk Extended Color BASIC
/// 1.1 `SAVEM`.
const TRAILER_MARKER: u8 = 0xFF;
/// Marker, big-endian count, and big-endian address.
const RECORD_HEADER_LEN: usize = 5;
/// DECB encodes a 64 KiB block with a zero 16-bit count.
const ZERO_COUNT_LEN: usize = u16::MAX as usize + 1;

impl DecbBinary {
    /// Parse a canonical DECB binary completely before returning any blocks.
    ///
    /// Disk Extended Color BASIC 1.1 `SAVEM` (`disk11.rom`,
    /// `$CF87-$CFAE`) writes repeated `$00`, big-endian count, big-endian
    /// address, and payload records followed by `$FF`, `$0000`, and a
    /// big-endian execution address. A zero data count represents 65,536
    /// bytes. This parser requires that canonical trailer and rejects bytes
    /// after it.
    pub fn parse(input: &[u8]) -> Result<Self, DecbError> {
        if input.is_empty() {
            return Err(error("DECB binary is empty"));
        }

        let mut offset = 0;
        let mut segments = Vec::new();
        loop {
            let header = record_header(input, offset)?;
            let count = u16::from_be_bytes([header[1], header[2]]);
            let address = u16::from_be_bytes([header[3], header[4]]);
            match header[0] {
                DATA_MARKER => {
                    let length = if count == 0 {
                        ZERO_COUNT_LEN
                    } else {
                        usize::from(count)
                    };
                    let payload_start = offset + RECORD_HEADER_LEN;
                    let payload_end = payload_start + length;
                    let payload = input.get(payload_start..payload_end).ok_or_else(|| {
                        error(format!(
                            "truncated DECB data block at offset {offset}: expected {length} byte(s)"
                        ))
                    })?;
                    segments.push(DecbSegment {
                        address,
                        bytes: payload.to_vec(),
                    });
                    offset = payload_end;
                    if offset == input.len() {
                        return Err(error("DECB binary is missing its execution trailer"));
                    }
                }
                TRAILER_MARKER => {
                    if count != 0 {
                        return Err(error(format!(
                            "nonzero DECB trailer count {count} at offset {offset}"
                        )));
                    }
                    offset += RECORD_HEADER_LEN;
                    if offset != input.len() {
                        return Err(error(format!(
                            "{} trailing byte(s) after DECB execution trailer",
                            input.len() - offset
                        )));
                    }
                    return Ok(Self {
                        segments,
                        exec_address: address,
                    });
                }
                marker => {
                    return Err(error(format!(
                        "unexpected DECB record marker ${marker:02X} at offset {offset}"
                    )));
                }
            }
        }
    }
}

fn record_header(input: &[u8], offset: usize) -> Result<&[u8], DecbError> {
    input
        .get(offset..offset + RECORD_HEADER_LEN)
        .ok_or_else(|| error(format!("truncated DECB record header at offset {offset}")))
}

fn error(message: impl Into<String>) -> DecbError {
    DecbError(message.into())
}

#[cfg(test)]
#[path = "decb_test.rs"]
mod tests;
