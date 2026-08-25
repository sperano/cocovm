//! Restore (stage 2: payload + resolved media -> live [`Machine`]).

use std::fmt;

use crate::cart::Cart;
use crate::{Machine, drivewire, fdc, vhd};

use super::error::SnapshotError;
use super::payload::{
    MediaRef, MediaRefs, MediaSources, RestoreNote, RestoredMachine, SnapshotPayload,
};

/// Turns a decoded payload plus resolved media into a running [`Machine`],
/// running the numbered steps below in order (see each step's own doc for
/// why the order matters). Missing-media failures from steps 2-4/6-7 collect
/// across all of them so a caller can prompt for every file at once; a
/// wrong-shape file still fails immediately as [`SnapshotError::MediaShape`].
pub fn restore(
    payload: SnapshotPayload,
    sources: MediaSources,
) -> Result<RestoredMachine, SnapshotError> {
    let SnapshotPayload { media, mut machine } = payload;
    validate_payload_shape(&machine)?;

    let MediaSources {
        system_rom,
        cart_roms,
        disks,
        vhds,
        drivewire,
        tape,
    } = sources;

    let mut missing = Vec::new();
    restore_system_rom(&mut machine, &media, system_rom, &mut missing);
    restore_cart_roms(&mut machine, &media, cart_roms, &mut missing)?;
    restore_disks(&mut machine, &media, disks, &mut missing)?;
    restore_vhds(&mut machine, &media, vhds, &mut missing)?;
    restore_drivewire(&mut machine, &media, drivewire, &mut missing)?;
    restore_tape(&mut machine, &media, tape, &mut missing)?;

    if !missing.is_empty() {
        return Err(SnapshotError::MissingMedia {
            descriptions: missing,
        });
    }

    // Must run after restore_disks: JvcDisk::data is skipped until
    // reattached, else this false-flags every transfer.
    validate_restored_disk_transfers(&mut machine)?;

    machine.after_restore();
    let notes = standing_notes(&mut machine);
    Ok(RestoredMachine { machine, notes })
}

/// Restore step 1: rejects a payload whose config is invalid, whose RAM
/// doesn't match the size the config declares, or whose cart tree contains
/// shapes deserialization alone can't catch (a nested Multi-Pak, or a
/// device index/cursor/cap field that would otherwise panic once the
/// machine runs — see [`crate::SystemBus::validate_restored`]).
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
    machine
        .bus
        .validate_restored()
        .map_err(SnapshotError::InvalidPayload)?;
    Ok(())
}

/// Restore step 5: bound-checks an in-flight WD1773 sector transfer against
/// the drive it currently targets, for any FD-502 reachable from the cart
/// tree (see [`crate::fdc::DiskCart::validate_restored_transfer`]).
fn validate_restored_disk_transfers(machine: &mut Machine) -> Result<(), SnapshotError> {
    let Some(disk_cart) = machine.bus.cart.as_disk_cart() else {
        return Ok(());
    };
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

/// Restore step 3: walks every cartridge slot ([`Cart::slots_mut`]) and
/// reattaches the ROM image for each variant that carries one. DeluxeRS232's
/// EPROM is optional (the pak works ROM-less) and only required when
/// `media.cart_roms` recorded one for its slot; every other ROM-bearing
/// type always requires a source.
fn restore_cart_roms(
    machine: &mut Machine,
    media: &MediaRefs,
    mut cart_roms: Vec<(Option<u8>, Vec<u8>)>,
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    for (mpi_slot, cart) in machine.bus.cart.slots_mut() {
        match cart {
            Cart::ROMPak(pak) => {
                require_cart_rom(mpi_slot, "ROMPak", media, &mut cart_roms, missing, |b| {
                    pak.reattach_image(b)
                })?
            }
            Cart::BankedROMPak(pak) => require_cart_rom(
                mpi_slot,
                "BankedROMPak",
                media,
                &mut cart_roms,
                missing,
                |b| pak.reattach_image(b),
            )?,
            Cart::GamesMasterCartridge(gmc) => require_cart_rom(
                mpi_slot,
                "GamesMasterCartridge",
                media,
                &mut cart_roms,
                missing,
                |b| gmc.reattach_rom(b),
            )?,
            Cart::DiskCart(disk) => {
                require_cart_rom(mpi_slot, "DiskCart", media, &mut cart_roms, missing, |b| {
                    disk.reattach_rom(b)
                })?
            }
            Cart::Orch90(orch) => {
                require_cart_rom(mpi_slot, "Orch90", media, &mut cart_roms, missing, |b| {
                    orch.reattach_rom(b)
                })?
            }
            Cart::DeluxeRS232(rs232) => {
                // Optional: see this function's doc comment.
                if let Some(bytes) = take_cart_rom(&mut cart_roms, mpi_slot) {
                    rs232.set_eprom(&bytes);
                }
            }
            Cart::Empty(_) | Cart::SoundSpeechCartridge(_) | Cart::DistoRTC(_) => {} // no ROM
            // Never produced by slots_mut (yields inner slots, not itself) or deserialization.
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

/// Reattaches a mandatory ROM-bearing cart's image, or records a `missing`
/// entry if `slot` has no source. A shape error from `reattach` (e.g. an
/// oversized image) fails immediately as [`SnapshotError::MediaShape`].
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
    missing_desc(
        &format!("{role} ROM"),
        &format!("in {}", slot_label(mpi_slot)),
        found.map(|r| &r.rom),
    )
}

/// Guards against a hand-edited or future-schema payload carrying more
/// [`MediaRefs`] entries than this build's hardware has drives for `role` —
/// anything beyond `capacity` is unreachable by any drive index this build loops over.
fn check_media_ref_capacity(
    refs: &[Option<MediaRef>],
    capacity: usize,
    role: &str,
) -> Result<(), SnapshotError> {
    if refs.len() > capacity {
        return Err(SnapshotError::InvalidPayload(format!(
            "snapshot records {} {role} media references, but this build only has {capacity} drives",
            refs.len()
        )));
    }
    Ok(())
}

/// Restore step 4: reattaches file bytes for every drive the FD-502 came
/// back with a `JvcDisk` in. Unlike the VHD/DriveWire step, drive occupancy
/// is read from the deserialized tree itself — `media.disks` is only
/// consulted here for the path in a missing-media message.
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
        let Some(disk) = disk_cart.disk_mut(i) else {
            continue;
        };
        let media_ref = media.disks.get(i).and_then(Option::as_ref);
        match slot.take() {
            Some(bytes) => disk
                .reattach_data(bytes)
                .map_err(|e| SnapshotError::MediaShape {
                    role: format!("floppy in drive {i}"),
                    detail: e.to_string(),
                })?,
            None => missing.push(missing_desc("floppy", &format!("in drive {i}"), media_ref)),
        }
    }
    Ok(())
}

/// Restore step 6 (VHD half): unlike floppies, `VHDDrive::image` is entirely
/// `#[serde(skip)]`, so mounted-ness can only be read from `media.vhds`.
fn restore_vhds(
    machine: &mut Machine,
    media: &MediaRefs,
    mut vhds: [Option<vhd::VHDImage>; vhd::DRIVE_COUNT],
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    check_media_ref_capacity(&media.vhds, vhd::DRIVE_COUNT, "VHD")?;
    for (i, slot) in vhds.iter_mut().enumerate() {
        let Some(media_ref) = media.vhds.get(i).and_then(Option::as_ref) else {
            continue;
        };
        match slot.take() {
            Some(image) => machine.bus.vhd.reattach_image(i, image),
            None => missing.push(missing_desc(
                "VHD",
                &format!("in drive {i}"),
                Some(media_ref),
            )),
        }
    }
    Ok(())
}

/// Restore step 6 (DriveWire half): same mounted-ness-only-in-`media` shape
/// as [`restore_vhds`]. If `media` says a drive was mounted but the Becker
/// port is disabled, that's a self-contradictory payload —
/// [`SnapshotError::InvalidPayload`], not a missing file.
fn restore_drivewire(
    machine: &mut Machine,
    media: &MediaRefs,
    mut drivewire: [Option<drivewire::DWImage>; drivewire::DRIVE_COUNT],
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    check_media_ref_capacity(&media.drivewire, drivewire::DRIVE_COUNT, "DriveWire")?;
    for (i, slot) in drivewire.iter_mut().enumerate() {
        let Some(media_ref) = media.drivewire.get(i).and_then(Option::as_ref) else {
            continue;
        };
        let Some(image) = slot.take() else {
            missing.push(missing_desc(
                "DriveWire image",
                &format!("in drive {i}"),
                Some(media_ref),
            ));
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

/// Restore step 7: reattach the tape, if `media.tape` says one was mounted.
fn restore_tape(
    machine: &mut Machine,
    media: &MediaRefs,
    tape: Option<Vec<u8>>,
    missing: &mut Vec<String>,
) -> Result<(), SnapshotError> {
    let Some(media_ref) = &media.tape else {
        return Ok(());
    };
    match tape {
        Some(bytes) => {
            machine
                .bus
                .cassette
                .reattach_tape(bytes)
                .map_err(|detail| SnapshotError::MediaShape {
                    role: "tape".to_string(),
                    detail,
                })
        }
        None => {
            missing.push(missing_desc("tape", "", Some(media_ref)));
            Ok(())
        }
    }
}

/// Restore step 9: collects notes for state a snapshot can't fully restore —
/// only reported when the tree actually shows the condition applies.
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

/// Builds one `missing` entry: the recorded path if a [`MediaRef`] exists,
/// or a "(no reference recorded)" variant if the snapshot never had one.
fn missing_desc(role: &str, location: &str, media_ref: Option<&MediaRef>) -> String {
    let sep = if location.is_empty() { "" } else { " " };
    match media_ref {
        Some(r) => format!("{role}{sep}{location}: {}", r.path.display()),
        None => format!("{role}{sep}{location} (no reference recorded in snapshot)"),
    }
}
