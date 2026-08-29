//! Container encode/decode: [`save`] turns a [`Machine`] + [`MediaRefs`]
//! into `.ccstate` bytes; [`load`] parses them back into a
//! [`SnapshotPayload`] (pure decode — no file I/O, no media resolution,
//! that's [`super::restore`]'s job).

use std::io::Read;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::Machine;

use super::error::SnapshotError;
use super::payload::{MediaRefs, SnapshotPayload, SnapshotPayloadRef};
use super::{CONTAINER_MAGIC, CONTAINER_VERSION, HEADER_LEN, SCHEMA_VERSION};

/// Cap on the inflated (decompressed CBOR) payload size that [`gunzip`] can
/// allocate, regardless of what a `.ccstate` file's gzip trailer claims —
/// gzip's own length field is attacker-controlled and not to be trusted
/// (a "decompression bomb": a tiny crafted file that inflates to gigabytes).
/// 64 MiB comfortably covers today's real ceiling — 2 MB max RAM
/// plus every other device's
/// state, cassette capture buffers, and DMP-105 paper-feed scratch — with
/// generous headroom for growth. Legitimate snapshots stay below this limit.
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

// ---- Save ---------------------------------------------------------------

/// Encodes `machine` + `media` into a `.ccstate` container (CBOR, gzipped,
/// with header). Caller must flush dirty media before hashing — see module doc.
pub fn save(machine: &Machine, media: &MediaRefs) -> Result<Vec<u8>, SnapshotError> {
    // Cart::Custom is #[serde(skip)]; fail early with a clear error instead of ciborium's.
    if machine.bus.cart.contains_custom() {
        return Err(SnapshotError::CustomCartNotSnapshotable);
    }

    let payload = SnapshotPayloadRef { media, machine };
    let mut cbor = Vec::new();
    ciborium::into_writer(&payload, &mut cbor).map_err(|e| SnapshotError::Encode(e.to_string()))?;

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut gz, &cbor).map_err(|e| SnapshotError::Encode(e.to_string()))?;
    let compressed = gz
        .finish()
        .map_err(|e| SnapshotError::Encode(e.to_string()))?;

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(CONTAINER_MAGIC);
    out.push(CONTAINER_VERSION);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

// ---- Load (stage 1: bytes -> payload, no media touched) -----------------

/// The parsed container header: the schema it claims, and the gzip body
/// slice past the header.
struct Header<'a> {
    schema: u32,
    body: &'a [u8],
}

/// Verifies magic and [`CONTAINER_VERSION`], splitting off the schema field
/// and gzip body. Too-short or wrong-magic both mean "not a CoCo save state"
/// ([`SnapshotError::NotASnapshot`]).
fn parse_header(bytes: &[u8]) -> Result<Header<'_>, SnapshotError> {
    if bytes.len() < HEADER_LEN || &bytes[..CONTAINER_MAGIC.len()] != CONTAINER_MAGIC {
        return Err(SnapshotError::NotASnapshot);
    }
    let version = bytes[CONTAINER_MAGIC.len()];
    if version != CONTAINER_VERSION {
        return Err(SnapshotError::UnsupportedContainer {
            found: version,
            supported: CONTAINER_VERSION,
        });
    }
    let schema_bytes: [u8; 4] = bytes[CONTAINER_MAGIC.len() + 1..HEADER_LEN]
        .try_into()
        .expect("slice is exactly 4 bytes by construction");
    Ok(Header {
        schema: u32::from_le_bytes(schema_bytes),
        body: &bytes[HEADER_LEN..],
    })
}

/// Inflates the gzip body, capped at [`MAX_PAYLOAD_BYTES`] to block
/// decompression bombs. Hitting the cap with more data still available is
/// [`SnapshotError::InvalidPayload`], not a truncated decode.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let mut out = Vec::new();
    let mut limited = GzDecoder::new(bytes).take(MAX_PAYLOAD_BYTES);
    limited
        .read_to_end(&mut out)
        .map_err(|e| SnapshotError::Decode(e.to_string()))?;
    if out.len() as u64 == MAX_PAYLOAD_BYTES {
        let mut probe = [0u8; 1];
        let more = limited
            .into_inner()
            .read(&mut probe)
            .map_err(|e| SnapshotError::Decode(e.to_string()))?;
        if more > 0 {
            return Err(SnapshotError::InvalidPayload(
                "payload exceeds MAX_PAYLOAD_BYTES".to_string(),
            ));
        }
    }
    Ok(out)
}

fn decode_payload(cbor: &[u8]) -> Result<SnapshotPayload, SnapshotError> {
    ciborium::from_reader(cbor).map_err(|e| SnapshotError::Decode(e.to_string()))
}

/// Migration dispatch point for old schema versions; empty today (schema 1
/// is the only one that has existed). Register old-schema decoders here
/// when [`SCHEMA_VERSION`] bumps.
fn migrate(_old_schema: u32, _cbor: &[u8]) -> Option<Result<SnapshotPayload, SnapshotError>> {
    None
}

/// Decodes a `.ccstate` container's bytes into a [`SnapshotPayload`]. Pure
/// decode — no file I/O or media resolution; that's [`super::restore`]'s job.
pub fn load(bytes: &[u8]) -> Result<SnapshotPayload, SnapshotError> {
    let header = parse_header(bytes)?;
    // Reject newer schemas before decompressing, so a crafted file never pays to inflate its body.
    if header.schema > SCHEMA_VERSION {
        return Err(SnapshotError::SchemaTooNew {
            found: header.schema,
            current: SCHEMA_VERSION,
        });
    }
    let cbor = gunzip(header.body)?;
    // Only Equal/Less remain here; Greater already returned earlier.
    if header.schema == SCHEMA_VERSION {
        return decode_payload(&cbor);
    }
    migrate(header.schema, &cbor).unwrap_or_else(|| {
        Err(SnapshotError::NoMigration {
            found: header.schema,
            current: SCHEMA_VERSION,
        })
    })
}
