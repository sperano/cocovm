//! Save-state snapshot engine: the `.ccstate` container format, media
//! references, and the restore flow that turns a decoded payload plus
//! resolved media bytes back into a running [`Machine`]
//! (`docs/plan-save-states.md`, `docs/plan-machine-persistence.md`).
//!
//! ## Compatibility contract
//!
//! **A snapshot written today must load in every future version**
//! (`docs/plan-machine-persistence.md` "Snapshot compatibility contract",
//! user requirement 2026-07-16). The payload is CBOR (`ciborium`), not a
//! positional format like bincode/postcard: CBOR carries field names with
//! the data, so serde's evolution tools (`#[serde(default)]`/`alias`) work
//! across versions instead of every struct needing hand-rolled versioning.
//! RAM and other big buffers stay compact via `serde_bytes`/
//! [`crate::serde_util::byte_array`] rather than base64-in-JSON. The whole
//! payload is gzipped with `flate2`.
//!
//! Four evolution rules govern every change to a type that lives inside
//! [`SnapshotPayload`] (enforced in review, not by the compiler):
//!
//! 1. never remove or rename a serialized field without `#[serde(alias =
//!    "old_name")]` or a migration;
//! 2. every added field carries `#[serde(default = "...")]` whose default
//!    reproduces the *old* behaviour (a snapshot from before the field
//!    existed must load as if the field had always held that value);
//! 3. never change the meaning or units of an existing field — add a new
//!    field and migrate instead;
//! 4. enum variants may be added, never repurposed.
//!
//! What actually *guarantees* rule compliance, per the plan, is the
//! golden-fixture gate: every time [`SCHEMA_VERSION`] bumps, or a release is
//! cut, a real snapshot fixture (small RAM, mid-BASIC-program) is committed
//! under `crates/coco-core/tests/fixtures/snapshots/`, and a test loads every
//! committed fixture and runs the trace-continuation check from it. That test
//! (and its first fixture) is phase 3's job — this module only builds the
//! engine the gate exercises.
//!
//! ## Container format
//!
//! ```text
//! magic "CCSTATE" (7 bytes) | container_version: u8 | schema: u32 LE | gzip(CBOR payload)
//! ```
//!
//! [`CONTAINER_VERSION`] is the container/header layout itself (this module's
//! own framing); [`SCHEMA_VERSION`] is the *machine-tree* schema and is
//! bumped only on a semantic break serde's evolution tools can't express —
//! everything the four rules above can absorb should NOT bump it. Recommended
//! file extension: `.ccstate` (a frontend concern; this module works on plain
//! bytes and never touches a file itself).
//!
//! ## Media: references, not content
//!
//! ROM/disk/VHD/DriveWire/tape bytes never travel inside the payload — they
//! can be copyrighted commercial software. [`MediaRefs`] records where the
//! frontend found each one (path) and a SHA-256 of its contents at save time;
//! [`load`] decodes the payload but does no file I/O, and [`restore`] takes
//! already-resolved [`MediaSources`] bytes/handles rather than opening
//! anything itself — the caller (a real frontend, or a test injecting bytes
//! directly) owns every filesystem access. The caller must flush dirty media
//! (unsaved floppy/tape changes) BEFORE calling [`sha256_file`]/[`save`], so
//! the recorded hash actually describes what's on disk — this module has no
//! flush hook of its own.
//!
//! The "no media bytes embedded" rule above is about WHOLE media images —
//! disk/VHD/DriveWire/tape files, ROM images. It does NOT extend to a
//! device's own in-flight I/O buffers: a snapshot taken mid-sector-transfer
//! carries that sector's bytes in the WD1773's `Transfer.buf` (see
//! `crate::wd1773`), the same way RAM carries whatever a program loaded into
//! it. Those bytes are as much machine state as a CPU register — only the
//! backing media file itself is excluded.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cart::Cart;
use crate::drivewire::DwImage;
use crate::vhd::VhdImage;
use crate::{Machine, drivewire, fdc, vhd};

/// Container magic bytes: the first 7 bytes of every `.ccstate` file.
pub const CONTAINER_MAGIC: &[u8; 7] = b"CCSTATE";
/// Container/header layout version (this module's own framing) — distinct
/// from [`SCHEMA_VERSION`], which versions the machine tree the container
/// carries.
pub const CONTAINER_VERSION: u8 = 1;
/// Machine-tree schema version. Bump ONLY on a semantic break the four
/// evolution rules in the module doc can't express; every other change
/// (added/renamed/removed fields, new enum variants) stays on the current
/// schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Byte length of the container header: magic + version byte + schema `u32`.
const HEADER_LEN: usize = CONTAINER_MAGIC.len() + 1 + 4;

/// Read buffer size for [`sha256_file`]'s streamed hash — large media files
/// (VHDs can run to hundreds of MB) must never be read whole into memory
/// just to hash them.
const SHA256_READ_BUF_LEN: usize = 8 * 1024;

// ---- Payload types ----------------------------------------------------

/// Everything a snapshot needs besides resolved media bytes: the machine
/// tree (config travels inside `machine.config`) plus where its media came
/// from.
#[derive(Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub media: MediaRefs,
    pub machine: Machine,
}

/// Borrowing twin of [`SnapshotPayload`] with identical field names/layout,
/// so [`save`] can CBOR-encode by reference instead of cloning the whole
/// machine tree just to hand it to `ciborium::into_writer`.
#[derive(Serialize)]
struct SnapshotPayloadRef<'a> {
    media: &'a MediaRefs,
    machine: &'a Machine,
}

/// Where one media file lived and what it hashed to, at save time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    /// As the frontend knew it at save time — absolute or relative, whatever
    /// the frontend itself used; this module never resolves or interprets
    /// it, only carries it.
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the file's contents at save time (see
    /// [`sha256_file`]).
    pub sha256: String,
}

/// A ROM-bearing cartridge's image reference, located by where it plugs in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotRomRef {
    /// `None` = the machine's own cartridge port; `Some(i)` = Multi-Pak slot
    /// `i` (0-3).
    pub mpi_slot: Option<u8>,
    pub rom: MediaRef,
}

/// Every media reference a snapshot might carry. Every field is
/// `#[serde(default)]` per evolution rule 2 — a future field added here must
/// still load an older snapshot as "this media slot was never used".
///
/// `disks`/`vhds`/`drivewire` are `Vec`, not `[Option<MediaRef>;
/// N::DRIVE_COUNT]`: a fixed-size array bakes today's `DRIVE_COUNT` into the
/// serialized shape, so a future change to it would fail to deserialize (or
/// silently truncate) every snapshot written before the change — the
/// evolution contract above forbids that. [`restore`] matches these up
/// against the machine's actual drive count itself (zip-style: a short `Vec`
/// leaves trailing drives as "never mounted"; a `Vec` longer than the current
/// build's `DRIVE_COUNT` is [`SnapshotError::InvalidPayload`], naming the
/// slot).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaRefs {
    #[serde(default)]
    pub system_rom: Option<MediaRef>,
    /// ROM-bearing carts, keyed by where they sit. Covers RomPak/
    /// BankedRomPak/Gmc/DiskCart/Orch90 images and the DeluxeRs232 EPROM —
    /// one entry per ROM-bearing cart that actually has an image (the
    /// DeluxeRs232 is the one cart in this list that can legitimately run
    /// without one; see [`restore`]'s cart-ROM step).
    #[serde(default)]
    pub cart_roms: Vec<SlotRomRef>,
    /// FD-502 JVC drives, indexed by drive number.
    #[serde(default)]
    pub disks: Vec<Option<MediaRef>>,
    /// VHD drives, indexed by drive number.
    #[serde(default)]
    pub vhds: Vec<Option<MediaRef>>,
    /// DriveWire drives, indexed by drive number.
    #[serde(default)]
    pub drivewire: Vec<Option<MediaRef>>,
    #[serde(default)]
    pub tape: Option<MediaRef>,
}

/// Resolved media bytes/handles for [`restore`], produced by the caller from
/// a [`MediaRefs`] (a real frontend re-reads each `path`; tests inject bytes
/// directly). Never touched by [`load`] — only [`restore`] consumes this.
#[derive(Default)]
pub struct MediaSources {
    pub system_rom: Option<Box<[u8]>>,
    /// `(mpi_slot, bytes)` pairs, matched against the deserialized cart
    /// tree's own `(mpi_slot, ..)` positions — see [`SlotRomRef`].
    pub cart_roms: Vec<(Option<u8>, Vec<u8>)>,
    pub disks: [Option<Vec<u8>>; fdc::DRIVE_COUNT],
    pub vhds: [Option<VhdImage>; vhd::DRIVE_COUNT],
    pub drivewire: [Option<DwImage>; drivewire::DRIVE_COUNT],
    pub tape: Option<Vec<u8>>,
}

/// The result of a successful [`restore`]: the live machine plus any
/// non-fatal notes the frontend should surface (e.g. as a toast). Hash
/// verification is caller-side (see [`MediaRef::verify`]) — a mismatch
/// warning is built there, not here.
pub struct RestoredMachine {
    pub machine: Machine,
    pub notes: Vec<RestoreNote>,
}

/// A non-fatal condition [`restore`] leaves for the caller to surface: state
/// that came back in a documented placeholder form rather than fully
/// restored (`docs/plan-save-states.md`). Typed rather than raw strings so a
/// caller can react to a specific condition programmatically — e.g. the egui
/// frontend re-injects the Disto RTC's host time source right after
/// `restore` returns and then drops [`RestoreNote::RtcPlaceholderTime`]
/// before showing the rest as a toast, since that note is only true for a
/// caller that DOESN'T immediately do that (a headless tool, a test) — a
/// caller matching on message text couldn't single that one note out safely
/// across future wording changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreNote {
    /// Print capture was active when this snapshot was saved; capture is
    /// stopped until restarted from the frontend.
    PrintCaptureStopped,
    /// The Deluxe RS-232 host connection restored as loopback; the real
    /// endpoint needs to be re-plugged from the frontend.
    Rs232EndpointLoopback,
    /// The Disto real-time clock restored to a placeholder time
    /// (1970-01-01); true only for a caller that doesn't itself re-sync it
    /// from a live time source right after restoring — see this type's own
    /// doc comment.
    RtcPlaceholderTime,
}

impl fmt::Display for RestoreNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreNote::PrintCaptureStopped => write!(
                f,
                "print capture was active when this snapshot was saved; capture is stopped until \
                 restarted from the Machine menu"
            ),
            RestoreNote::Rs232EndpointLoopback => write!(
                f,
                "Deluxe RS-232 host connection restored as loopback; re-plug the real endpoint from \
                 the frontend"
            ),
            RestoreNote::RtcPlaceholderTime => write!(
                f,
                "Disto real-time clock restored to a placeholder time (1970-01-01); re-sync it from \
                 the frontend"
            ),
        }
    }
}

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

/// Cap on the inflated (decompressed CBOR) payload size [`gunzip`] will ever
/// allocate, regardless of what a `.ccstate` file's gzip header claims —
/// gzip's own length field is attacker-controlled and not to be trusted
/// (a "decompression bomb": a tiny crafted file that inflates to gigabytes).
/// 64 MiB comfortably covers today's real ceiling — 2 MB max RAM
/// (`docs/plan-save-states.md` "2048K stock GIME") plus every other device's
/// state, cassette capture buffers, and DMP-105 paper-feed scratch — with
/// generous headroom for growth; nothing legitimate should ever come close.
const MAX_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

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
/// decode, no file I/O and no media resolution — that's [`restore`]'s job,
/// once the caller has turned this payload's [`MediaRefs`] into
/// [`MediaSources`].
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

// ---- Restore (stage 2: payload + resolved media -> live Machine) --------

/// Turn a decoded payload plus resolved media into a running [`Machine`],
/// per the restore order documented on each private step below:
///
/// 1. validate `machine.config`/RAM length/cart-tree shape/device
///    index-cursor-cap fields (a corrupted/hand-edited payload must error,
///    not panic, later — see [`validate_payload_shape`]);
/// 2. reattach the system ROM;
/// 3. reattach every ROM-bearing cartridge's image;
/// 4. reattach every mounted floppy;
/// 5. bound-check any in-flight WD1773 sector transfer against the drive it
///    targets, now that step 4 has reattached its data (see
///    [`validate_restored_disk_transfers`] — this can't run any earlier);
/// 6. reattach every mounted VHD/DriveWire image;
/// 7. reattach the tape, if one was mounted;
/// 8. [`Machine::after_restore`];
/// 9. collect standing notes for state that came back in a placeholder form.
///
/// Missing-media failures from steps 2-4 and 6-7 are collected across all of
/// them rather than stopping at the first one, so a caller can prompt for every
/// missing file at once instead of one at a time; a *wrong-shape* media file
/// (e.g. a floppy whose geometry no longer matches) still fails immediately
/// with [`SnapshotError::MediaShape`], since retrying that one file wouldn't
/// help either.
pub fn restore(
    payload: SnapshotPayload,
    sources: MediaSources,
) -> Result<RestoredMachine, SnapshotError> {
    let SnapshotPayload { media, mut machine } = payload;
    validate_payload_shape(&machine)?;

    let MediaSources { system_rom, cart_roms, disks, vhds, drivewire, tape } = sources;

    let mut missing = Vec::new();
    restore_system_rom(&mut machine, &media, system_rom, &mut missing);
    restore_cart_roms(&mut machine, &media, cart_roms, &mut missing)?;
    restore_disks(&mut machine, &media, disks, &mut missing)?;
    restore_vhds(&mut machine, &media, vhds, &mut missing)?;
    restore_drivewire(&mut machine, &media, drivewire, &mut missing)?;
    restore_tape(&mut machine, &media, tape, &mut missing)?;

    if !missing.is_empty() {
        return Err(SnapshotError::MissingMedia { descriptions: missing });
    }

    // Only reachable once every floppy the payload needed has been
    // reattached (`restore_disks`, just above) — `JvcDisk::data` is
    // `#[serde(skip)]`, empty until then, so any earlier check would flag
    // every in-flight sector transfer as out of bounds, not just corrupted
    // ones.
    validate_restored_disk_transfers(&mut machine)?;

    machine.after_restore();
    let notes = standing_notes(&mut machine);
    Ok(RestoredMachine { machine, notes })
}

/// Restore step 1: reject a payload whose declared config is internally
/// invalid, or whose RAM doesn't match the size that config declares —
/// either means the payload was hand-edited or corrupted, and every later
/// step assumes both hold (e.g. physical-address masking against
/// `bus.ram.len()`). Also rejects two shapes deserialization alone can't:
/// a nested Multi-Pak (not valid hardware, and would bypass
/// [`Cart::slots_mut`]'s ROM-reattachment walk entirely — see
/// [`Cart::contains_nested_multipak`]) and any device-level index/cursor/cap
/// field that would panic Rust's own bounds checks once the machine runs
/// (see [`SystemBus::validate_restored`]).
fn validate_payload_shape(machine: &Machine) -> Result<(), SnapshotError> {
    machine
        .config
        .validate()
        .map_err(|e| SnapshotError::InvalidPayload(format!("invalid machine config: {e}")))?;
    let expected = machine.config.memory.bytes();
    if machine.bus.ram.len() != expected {
        return Err(SnapshotError::InvalidPayload(format!(
            "snapshot RAM is {} bytes, but {:?} needs {expected}",
            machine.bus.ram.len(),
            machine.config.memory
        )));
    }
    if machine.bus.cart.contains_nested_multipak() {
        return Err(SnapshotError::InvalidPayload(
            "nested Multi-Pak is not valid hardware".to_string(),
        ));
    }
    machine.bus.validate_restored().map_err(SnapshotError::InvalidPayload)?;
    Ok(())
}

/// Restore step 5: bound-check an in-flight WD1773 sector transfer against
/// the drive it currently targets, for the FD-502 reachable from the cart
/// tree (direct port or nested one level in a Multi-Pak — same reach as
/// [`Cart::as_disk_cart`]). See [`crate::fdc::DiskCart::validate_restored_transfer`].
fn validate_restored_disk_transfers(machine: &mut Machine) -> Result<(), SnapshotError> {
    let Some(disk_cart) = machine.bus.cart.as_disk_cart() else { return Ok(()) };
    disk_cart
        .validate_restored_transfer()
        .map_err(|e| SnapshotError::InvalidPayload(format!("FD-502: {e}")))
}

/// Restore step 2: the system ROM is required whenever the tree needs one —
/// always, there is no ROM-less CoCo.
fn restore_system_rom(
    machine: &mut Machine,
    media: &MediaRefs,
    system_rom: Option<Box<[u8]>>,
    missing: &mut Vec<String>,
) {
    match system_rom {
        Some(bytes) => machine.bus.reattach_rom(bytes),
        None => missing.push(missing_desc("system ROM", "", media.system_rom.as_ref())),
    }
}

/// Restore step 3: walk every cartridge slot ([`Cart::slots_mut`]) and
/// reattach the ROM image for each variant that carries one.
///
/// The DeluxeRs232 is handled differently from the other five ROM-bearing
/// types: `RomPak`/`BankedRomPak`/`Gmc`/`DiskCart`/`Orch90` can only exist in
/// the tree at all by having been built from a nonempty image
/// (`from_bytes`/`new` reject an empty one), so their presence always means a
/// `MediaRefs` entry was recorded and a source is required. A DeluxeRs232's
/// EPROM is optional even at construction (`docs/plan-deluxe-rs232.md`: "the
/// pak works ROM-less"), and that optionality can't be recovered from the
/// deserialized tree — `eprom` is itself `#[serde(skip)]` and always comes
/// back `None` regardless of whether one was mounted. So a DeluxeRs232 only
/// requires its source when `media.cart_roms` actually recorded one for its
/// slot; if it didn't, this cart legitimately runs ROM-less and nothing is
/// missing.
fn restore_cart_roms(
    machine: &mut Machine,
    media: &MediaRefs,
    mut cart_roms: Vec<(Option<u8>, Vec<u8>)>,
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    for (mpi_slot, cart) in machine.bus.cart.slots_mut() {
        match cart {
            Cart::RomPak(pak) => require_cart_rom(mpi_slot, "RomPak", media, &mut cart_roms, missing, |b| {
                pak.reattach_image(b)
            })?,
            Cart::BankedRomPak(pak) => {
                require_cart_rom(mpi_slot, "BankedRomPak", media, &mut cart_roms, missing, |b| {
                    pak.reattach_image(b)
                })?
            }
            Cart::Gmc(gmc) => require_cart_rom(mpi_slot, "Gmc", media, &mut cart_roms, missing, |b| {
                gmc.reattach_rom(b)
            })?,
            Cart::DiskCart(disk) => {
                require_cart_rom(mpi_slot, "DiskCart", media, &mut cart_roms, missing, |b| {
                    disk.reattach_rom(b)
                })?
            }
            Cart::Orch90(orch) => require_cart_rom(mpi_slot, "Orch90", media, &mut cart_roms, missing, |b| {
                orch.reattach_rom(b)
            })?,
            Cart::DeluxeRs232(rs232) => {
                // Optional: see this function's doc comment.
                if let Some(bytes) = take_cart_rom(&mut cart_roms, mpi_slot) {
                    rs232.set_eprom(&bytes);
                }
            }
            Cart::Empty(_) | Cart::Ssc(_) | Cart::DistoRtc(_) => {} // no ROM
            // Never produced by `slots_mut` (a `MultiPak`'s own slots are
            // what it yields, not itself) and never produced by
            // deserialization (`#[serde(skip)]`), respectively.
            Cart::MultiPak(_) | Cart::Custom(_) => {}
        }
    }
    Ok(())
}

/// Take and remove the ROM bytes recorded for `slot` from `cart_roms`, if
/// any.
fn take_cart_rom(cart_roms: &mut Vec<(Option<u8>, Vec<u8>)>, slot: Option<u8>) -> Option<Vec<u8>> {
    let idx = cart_roms.iter().position(|(s, _)| *s == slot)?;
    Some(cart_roms.remove(idx).1)
}

/// Reattach a mandatory ROM-bearing cart's image: pop its bytes out of
/// `cart_roms` and hand them to `reattach`, recording a `missing` entry
/// instead if there's no source for `slot`. A shape error from `reattach`
/// itself (e.g. an oversized image) is returned immediately as
/// [`SnapshotError::MediaShape`] — unlike a missing source, a bad source
/// isn't something batching more `missing` entries would help with.
fn require_cart_rom<E: fmt::Display>(
    mpi_slot: Option<u8>,
    role: &str,
    media: &MediaRefs,
    cart_roms: &mut Vec<(Option<u8>, Vec<u8>)>,
    missing: &mut Vec<String>,
    reattach: impl FnOnce(&[u8]) -> Result<(), E>,
) -> Result<(), SnapshotError> {
    match take_cart_rom(cart_roms, mpi_slot) {
        Some(bytes) => reattach(&bytes).map_err(|e| SnapshotError::MediaShape {
            role: format!("{role} ROM ({})", slot_label(mpi_slot)),
            detail: e.to_string(),
        }),
        None => {
            missing.push(missing_cart_rom_desc(role, mpi_slot, media));
            Ok(())
        }
    }
}

fn slot_label(mpi_slot: Option<u8>) -> String {
    match mpi_slot {
        None => "the cartridge port".to_string(),
        Some(i) => format!("Multi-Pak slot {i}"),
    }
}

fn missing_cart_rom_desc(role: &str, mpi_slot: Option<u8>, media: &MediaRefs) -> String {
    let found = media.cart_roms.iter().find(|r| r.mpi_slot == mpi_slot);
    missing_desc(&format!("{role} ROM"), &format!("in {}", slot_label(mpi_slot)), found.map(|r| &r.rom))
}

/// Guard for [`MediaRefs::disks`]/[`vhds`](MediaRefs::vhds)/
/// [`drivewire`](MediaRefs::drivewire): they're `Vec`, not a fixed
/// `[Option<MediaRef>; DRIVE_COUNT]` array (see [`MediaRefs`]'s doc comment
/// for why), so a hand-edited — or genuinely future-schema — payload can
/// still carry more entries than this build's hardware has drives for.
/// Anything beyond `capacity` is unreachable by any drive index this build
/// will ever loop over, so it's flagged here rather than silently ignored.
fn check_media_ref_capacity(refs: &[Option<MediaRef>], capacity: usize, role: &str) -> Result<(), SnapshotError> {
    if refs.len() > capacity {
        return Err(SnapshotError::InvalidPayload(format!(
            "snapshot records {} {role} media references, but this build only has {capacity} drives",
            refs.len()
        )));
    }
    Ok(())
}

/// Restore step 4: for every drive the FD-502 (wherever it's plugged in)
/// came back with a `JvcDisk` in, reattach that drive's file bytes.
///
/// Unlike the VHD/DriveWire step below, a mounted floppy's *presence* is
/// directly visible in the deserialized tree: `JvcDisk::data` is skipped,
/// but the `Option<JvcDisk>` wrapping it is an ordinary field, so
/// `DiskCart::disk_mut` already reports the right drives as occupied without
/// consulting `media` at all — `media.disks` is only needed here for the
/// path in a missing-media message. `media.disks` being a `Vec` (see
/// [`MediaRefs`]'s doc comment): a short one leaves trailing drives'
/// `media_ref` at `None` via [`<[_]>::get`] — same as "no reference recorded
/// in snapshot" for a drive `MediaRefs` never had a field for at all.
fn restore_disks(
    machine: &mut Machine,
    media: &MediaRefs,
    mut disks: [Option<Vec<u8>>; fdc::DRIVE_COUNT],
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    check_media_ref_capacity(&media.disks, fdc::DRIVE_COUNT, "disk")?;
    let Some(disk_cart) = machine.bus.cart.as_disk_cart() else {
        return Ok(());
    };
    for (i, slot) in disks.iter_mut().enumerate() {
        let Some(disk) = disk_cart.disk_mut(i) else { continue };
        let media_ref = media.disks.get(i).and_then(Option::as_ref);
        match slot.take() {
            Some(bytes) => disk.reattach_data(bytes).map_err(|e| SnapshotError::MediaShape {
                role: format!("floppy in drive {i}"),
                detail: e.to_string(),
            })?,
            None => missing.push(missing_desc("floppy", &format!("in drive {i}"), media_ref)),
        }
    }
    Ok(())
}

/// Restore step 5 (VHD half): unlike floppies, a `VhdDrive::image` is
/// *entirely* `#[serde(skip)]`, so the deserialized tree always looks
/// unmounted — whether a drive was mounted at save time can only be read
/// from `media.vhds`, per the phase-2 spec.
fn restore_vhds(
    machine: &mut Machine,
    media: &MediaRefs,
    mut vhds: [Option<VhdImage>; vhd::DRIVE_COUNT],
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    check_media_ref_capacity(&media.vhds, vhd::DRIVE_COUNT, "VHD")?;
    for (i, slot) in vhds.iter_mut().enumerate() {
        let Some(media_ref) = media.vhds.get(i).and_then(Option::as_ref) else { continue };
        match slot.take() {
            Some(image) => machine.bus.vhd.reattach_image(i, image),
            None => missing.push(missing_desc("VHD", &format!("in drive {i}"), Some(media_ref))),
        }
    }
    Ok(())
}

/// Restore step 5 (DriveWire half): same "mounted-ness only lives in
/// `media`" shape as [`restore_vhds`]. If `media` says a DriveWire drive was
/// mounted but the restored tree has the Becker port disabled entirely
/// (`bus.drivewire == None`), that's not a missing *file* — it's the payload
/// contradicting itself — so this reports [`SnapshotError::InvalidPayload`]
/// instead of adding to `missing`.
fn restore_drivewire(
    machine: &mut Machine,
    media: &MediaRefs,
    mut drivewire: [Option<DwImage>; drivewire::DRIVE_COUNT],
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    check_media_ref_capacity(&media.drivewire, drivewire::DRIVE_COUNT, "DriveWire")?;
    for (i, slot) in drivewire.iter_mut().enumerate() {
        let Some(media_ref) = media.drivewire.get(i).and_then(Option::as_ref) else { continue };
        let Some(image) = slot.take() else {
            missing.push(missing_desc("DriveWire image", &format!("in drive {i}"), Some(media_ref)));
            continue;
        };
        match machine.bus.drivewire.as_mut() {
            Some(dw) => dw.reattach(i, image),
            None => {
                return Err(SnapshotError::InvalidPayload(format!(
                    "snapshot records a DriveWire image mounted in drive {i}, but its Becker \
                     port is disabled"
                )));
            }
        }
    }
    Ok(())
}

/// Restore step 6: reattach the tape, if `media.tape` says one was mounted.
fn restore_tape(
    machine: &mut Machine,
    media: &MediaRefs,
    tape: Option<Vec<u8>>,
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    let Some(media_ref) = &media.tape else { return Ok(()) };
    match tape {
        Some(bytes) => machine
            .bus
            .cassette
            .reattach_tape(bytes)
            .map_err(|detail| SnapshotError::MediaShape { role: "tape".to_string(), detail }),
        None => {
            missing.push(missing_desc("tape", "", Some(media_ref)));
            Ok(())
        }
    }
}

/// Restore step 8: state that a snapshot can't fully restore on its own and
/// comes back in a documented placeholder form instead — only reported when
/// the tree actually shows the condition applies, not unconditionally.
fn standing_notes(machine: &mut Machine) -> Vec<RestoreNote> {
    let mut notes = Vec::new();
    if machine.bus.bitbanger.capture_was_stopped_on_restore() {
        notes.push(RestoreNote::PrintCaptureStopped);
    }
    if machine.bus.cart.as_deluxe_rs232().is_some() {
        notes.push(RestoreNote::Rs232EndpointLoopback);
    }
    if machine.bus.cart.as_disto_rtc().is_some() {
        notes.push(RestoreNote::RtcPlaceholderTime);
    }
    notes
}

/// Build one `missing` entry: `"{role} {location}: {path}"` if a
/// [`MediaRef`] was recorded (even though its source wasn't provided), or a
/// "(no reference recorded)" variant if the snapshot never had one at all —
/// both are real states a hand-edited or partially-transferred snapshot
/// directory can be in.
fn missing_desc(role: &str, location: &str, media_ref: Option<&MediaRef>) -> String {
    let sep = if location.is_empty() { "" } else { " " };
    match media_ref {
        Some(r) => format!("{role}{sep}{location}: {}", r.path.display()),
        None => format!("{role}{sep}{location} (no reference recorded in snapshot)"),
    }
}

// ---- Hash helpers (file IO allowed here only) ----------------------------

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

/// Lowercase hex SHA-256 of the file at `path`, streamed in
/// [`SHA256_READ_BUF_LEN`]-byte chunks rather than read whole into memory —
/// VHD images can run to hundreds of MB.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; SHA256_READ_BUF_LEN];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    use fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Outcome of checking a [`MediaRef`] against the file it names, right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaCheck {
    /// The file exists and still hashes to `sha256`.
    Ok,
    /// The file exists but hashes to something else — the frontend should
    /// offer "load with warning", per `docs/plan-save-states.md`.
    Mismatch { actual: String },
    /// The file doesn't exist, or couldn't be read for any other reason —
    /// the frontend should treat this as an error (prompt to re-locate it).
    Missing,
}

impl MediaRef {
    /// Check this reference against the file it names on the current
    /// filesystem. Any read failure (missing file, permission error, ...) is
    /// reported as [`MediaCheck::Missing`] — from the caller's point of view
    /// "can't verify this" and "it's not there" call for the same response.
    pub fn verify(&self) -> MediaCheck {
        match sha256_file(&self.path) {
            Ok(actual) if actual == self.sha256 => MediaCheck::Ok,
            Ok(actual) => MediaCheck::Mismatch { actual },
            Err(_) => MediaCheck::Missing,
        }
    }
}

// ---- Errors ---------------------------------------------------------------

/// Everything that can go wrong across [`save`]/[`load`]/[`restore`], with a
/// [`Display`](fmt::Display) message precise enough to show a user directly.
#[derive(Debug)]
pub enum SnapshotError {
    /// The bytes don't start with [`CONTAINER_MAGIC`], or are too short to
    /// contain a full header at all.
    NotASnapshot,
    /// The container's own framing version isn't one this build understands.
    UnsupportedContainer { found: u8, supported: u8 },
    /// The machine-tree schema is newer than this build knows how to read.
    SchemaTooNew { found: u32, current: u32 },
    /// The machine-tree schema is older than current, and no migration is
    /// registered for it (see [`migrate`]).
    NoMigration { found: u32, current: u32 },
    /// CBOR encoding failed (besides the dedicated
    /// [`SnapshotError::CustomCartNotSnapshotable`] case).
    Encode(String),
    /// Gzip or CBOR decoding failed.
    Decode(String),
    /// The machine being saved has a [`Cart::Custom`] test double inserted,
    /// which has no serializable shape.
    CustomCartNotSnapshotable,
    /// One or more media sources needed by [`restore`] weren't provided;
    /// `descriptions` names every one collected, so a caller can prompt for
    /// all of them at once.
    MissingMedia { descriptions: Vec<String> },
    /// A provided media source doesn't fit the shape the snapshot recorded
    /// (wrong floppy geometry, oversized ROM image, ...) — retrying with the
    /// *same* file wouldn't help, unlike [`SnapshotError::MissingMedia`].
    MediaShape { role: String, detail: String },
    /// The decoded payload is internally invalid (bad config, RAM length
    /// mismatch, self-contradictory media state, ...) — never reached from
    /// bytes this module itself produced, only from corrupted or
    /// hand-edited ones.
    InvalidPayload(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::NotASnapshot => write!(f, "not a CoCo save state"),
            SnapshotError::UnsupportedContainer { found, supported } => write!(
                f,
                "unsupported save-state container version {found} (this build supports {supported})"
            ),
            SnapshotError::SchemaTooNew { found, current } => write!(
                f,
                "this save state was written by a newer version (schema {found}); this build \
                 understands up to schema {current}"
            ),
            SnapshotError::NoMigration { found, current } => write!(
                f,
                "this save state is schema {found}; this build is schema {current} and has no \
                 migration path from {found}"
            ),
            SnapshotError::Encode(msg) => write!(f, "failed to encode save state: {msg}"),
            SnapshotError::Decode(msg) => write!(f, "failed to decode save state: {msg}"),
            SnapshotError::CustomCartNotSnapshotable => {
                write!(f, "cannot save: an out-of-crate test cartridge is inserted")
            }
            SnapshotError::MissingMedia { descriptions } => {
                write!(f, "missing media needed to restore this save state:")?;
                for desc in descriptions {
                    write!(f, "\n  - {desc}")?;
                }
                Ok(())
            }
            SnapshotError::MediaShape { role, detail } => {
                write!(f, "{role} doesn't match this save state: {detail}")
            }
            SnapshotError::InvalidPayload(msg) => write!(f, "invalid save state: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotError {}
