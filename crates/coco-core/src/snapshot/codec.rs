//! Container encode/decode: [`save`] turns a [`Machine`] + [`MediaRefs`]
//! into `.ccstate` bytes; [`load`] parses them back into a
//! [`SnapshotPayload`] (pure decode — no file I/O, no media resolution,
//! that's [`super::restore`]'s job).

use std::io::Read;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::Machine;

use super::error::SnapshotError;
use super::payload::{MediaRefs, SnapshotPayload, SnapshotPayloadRef};
use super::{CONTAINER_MAGIC, CONTAINER_VERSION, HEADER_LEN, SCHEMA_VERSION};

/// Cap on the inflated (decompressed CBOR) payload size [`gunzip`] will ever
/// allocate, regardless of what a `.ccstate` file's gzip trailer claims —
/// gzip's own length field is attacker-controlled and not to be trusted
/// (a "decompression bomb": a tiny crafted file that inflates to gigabytes).
/// 64 MiB comfortably covers today's real ceiling — 2 MB max RAM
/// (`docs/plan-save-states.md` "2048K stock GIME") plus every other device's
/// state, cassette capture buffers, and DMP-105 paper-feed scratch — with
/// generous headroom for growth; nothing legitimate should ever come close.
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

// ---- Save ---------------------------------------------------------------

/// Encode `machine` + `media` into a `.ccstate` container: CBOR, gzipped,
/// with the container header prepended.
///
/// The caller must flush dirty media (unsaved floppy/tape edits) BEFORE
/// building `media`'s hashes and calling this — see the module doc. This
/// function does no file I/O of its own.
pub fn save(machine: &Machine, media: &MediaRefs) -> Result<Vec<u8>, SnapshotError> {
    // A `Cart::Custom` test double has no serializable shape (phase 1: its
    // variant is `#[serde(skip)]`). Detect it up front so the failure is a
    // clean, documented error instead of whatever ciborium's generated
    // "skipped variant" error happens to say.
    if machine.bus.cart.contains_custom() {
        return Err(SnapshotError::CustomCartNotSnapshotable);
    }

    let payload = SnapshotPayloadRef { media, machine };
    let mut cbor = Vec::new();
    ciborium::into_writer(&payload, &mut cbor).map_err(|e| SnapshotError::Encode(e.to_string()))?;

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut gz, &cbor).map_err(|e| SnapshotError::Encode(e.to_string()))?;
    let compressed = gz.finish().map_err(|e| SnapshotError::Encode(e.to_string()))?;

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

/// Verify the magic and [`CONTAINER_VERSION`], and split off the schema
/// field and gzip body. A file too short to even contain a full header is
/// [`SnapshotError::NotASnapshot`], same as a wrong magic — both mean "this
/// isn't (recognizably) a CoCo save state", as opposed to a container we
/// understand but whose *contents* we can't decode.
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

/// Inflate `bytes` (the gzip body past the container header), capped at
/// [`MAX_PAYLOAD_BYTES`] via [`std::io::Read::take`] so a crafted file can't
/// force an unbounded allocation before its (attacker-controlled) length is
/// ever checked against anything real. If the cap is hit exactly AND the
/// underlying stream still has more to give, that's a payload genuinely over
/// the cap (not a coincidentally-cap-sized legitimate one) —
/// [`SnapshotError::InvalidPayload`], not a truncated decode.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let mut out = Vec::new();
    let mut limited = GzDecoder::new(bytes).take(MAX_PAYLOAD_BYTES);
    limited.read_to_end(&mut out).map_err(|e| SnapshotError::Decode(e.to_string()))?;
    if out.len() as u64 == MAX_PAYLOAD_BYTES {
        let mut probe = [0u8; 1];
        let more = limited.into_inner().read(&mut probe).map_err(|e| SnapshotError::Decode(e.to_string()))?;
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

/// Schema-downgrade migration dispatch point. Today's table is empty —
/// schema 1 is the only schema that has ever existed — so every call falls
/// through to `None` and [`load`] reports [`SnapshotError::NoMigration`].
/// When a future breaking change bumps [`SCHEMA_VERSION`], register the old
/// schema number here with a function that decodes its CBOR shape and
/// upgrades it to the current one.
fn migrate(_old_schema: u32, _cbor: &[u8]) -> Option<Result<SnapshotPayload, SnapshotError>> {
    None
}

/// Decode a `.ccstate` container's bytes into a [`SnapshotPayload`]. Pure
/// decode, no file I/O and no media resolution — that's [`super::restore`]'s job,
/// once the caller has turned this payload's [`MediaRefs`] into
/// [`super::MediaSources`].
pub fn load(bytes: &[u8]) -> Result<SnapshotPayload, SnapshotError> {
    let header = parse_header(bytes)?;
    // Checked BEFORE decompressing: a schema newer than this build
    // understands is rejected outright, so a crafted file claiming one never
    // pays for (or risks) inflating its gzip body at all — [`gunzip`]'s own
    // [`MAX_PAYLOAD_BYTES`] cap is the second line of defense for the
    // schemas that DO proceed to decompression.
    if header.schema > SCHEMA_VERSION {
        return Err(SnapshotError::SchemaTooNew { found: header.schema, current: SCHEMA_VERSION });
    }
    let cbor = gunzip(header.body)?;
    // `header.schema > SCHEMA_VERSION` already returned above, so only
    // Equal/Less remain here.
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
