//! Restore (stage 2: payload + resolved media -> live [`Machine`]).

use std::fmt;

use crate::cart::Cart;
use crate::{drivewire, fdc, vhd, Machine};

use super::error::SnapshotError;
use super::payload::{MediaRef, MediaRefs, MediaSources, RestoreNote, RestoredMachine, SnapshotPayload};

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
/// (see [`crate::SystemBus::validate_restored`]).
pub(crate) fn validate_payload_shape(machine: &Machine) -> Result<(), SnapshotError> {
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
/// The DeluxeRS232 is handled differently from the other five ROM-bearing
/// types: `ROMPak`/`BankedROMPak`/`GamesMasterCartridge`/`DiskCart`/`Orch90`
/// can only exist in the tree at all by having been built from a nonempty
/// image (`from_bytes`/`new` reject an empty one), so their presence always
/// means a `MediaRefs` entry was recorded and a source is required. A
/// DeluxeRS232's EPROM is optional even at construction
/// (`docs/plan-deluxe-rs232.md`: "the pak works ROM-less"), and that
/// optionality can't be recovered from the deserialized tree — `eprom` is
/// itself `#[serde(skip)]` and always comes back `None` regardless of
/// whether one was mounted. So a DeluxeRS232 only requires its source when
/// `media.cart_roms` actually recorded one for its slot; if it didn't, this
/// cart legitimately runs ROM-less and nothing is missing.
fn restore_cart_roms(
    machine: &mut Machine,
    media: &MediaRefs,
    mut cart_roms: Vec<(Option<u8>, Vec<u8>)>,
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    for (mpi_slot, cart) in machine.bus.cart.slots_mut() {
        match cart {
            Cart::ROMPak(pak) => require_cart_rom(mpi_slot, "ROMPak", media, &mut cart_roms, missing, |b| {
                pak.reattach_image(b)
            })?,
            Cart::BankedROMPak(pak) => {
                require_cart_rom(mpi_slot, "BankedROMPak", media, &mut cart_roms, missing, |b| {
                    pak.reattach_image(b)
                })?
            }
            Cart::GamesMasterCartridge(gmc) => {
                require_cart_rom(mpi_slot, "GamesMasterCartridge", media, &mut cart_roms, missing, |b| {
                    gmc.reattach_rom(b)
                })?
            }
            Cart::DiskCart(disk) => {
                require_cart_rom(mpi_slot, "DiskCart", media, &mut cart_roms, missing, |b| {
                    disk.reattach_rom(b)
                })?
            }
            Cart::Orch90(orch) => require_cart_rom(mpi_slot, "Orch90", media, &mut cart_roms, missing, |b| {
                orch.reattach_rom(b)
            })?,
            Cart::DeluxeRS232(rs232) => {
                // Optional: see this function's doc comment.
                if let Some(bytes) = take_cart_rom(&mut cart_roms, mpi_slot) {
                    rs232.set_eprom(&bytes);
                }
            }
            Cart::Empty(_) | Cart::SoundSpeechCartridge(_) | Cart::DistoRTC(_) => {} // no ROM
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

/// Restore step 5 (VHD half): unlike floppies, a `VHDDrive::image` is
/// *entirely* `#[serde(skip)]`, so the deserialized tree always looks
/// unmounted — whether a drive was mounted at save time can only be read
/// from `media.vhds`, per the phase-2 spec.
fn restore_vhds(
    machine: &mut Machine,
    media: &MediaRefs,
    mut vhds: [Option<vhd::VHDImage>; vhd::DRIVE_COUNT],
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
    mut drivewire: [Option<drivewire::DWImage>; drivewire::DRIVE_COUNT],
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
        notes.push(RestoreNote::RS232EndpointLoopback);
    }
    if machine.bus.cart.as_disto_rtc().is_some() {
        notes.push(RestoreNote::RTCPlaceholderTime);
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
