//! Standalone [`MediaRef`]/[`MediaRefs`] helpers shared by the SAVE side
//! ([`super::save`]) and LOAD side ([`super::restore`]): hashing a file into
//! a ref, reading/opening a ref's file back with mismatch-warning bookkeeping,
//! plus the two small lookups used by [`crate::CocoApp::rebuild_cart_mirrors`]
//! to turn a restored cart tree back into path-bearing UI state.

use std::path::{Path, PathBuf};

use coco_core::cart::Cart;
use coco_core::snapshot::{self, CartROMRole, CartROMSource, MediaCheck, MediaRef, MediaRefs};

use crate::{
    Coco12ROMResult, MPISlot, ROM_DB_PSEUDO_PATH_PREFIX, compose_coco12_rom, installed_roms_dir,
};

/// The recorded `mpi_slot: None` cart-ROM path, if any — used for the
/// direct-port `cart_path`/`rs232_eprom_path` cases.
pub(super) fn direct_port_rom_path(media: &MediaRefs) -> Option<PathBuf> {
    media
        .cart_roms
        .iter()
        .find(|r| r.mpi_slot.is_none())
        .map(|r| r.rom.path.clone())
}

/// The [`MPISlot`] `cart` at slot `i` gets a display path from `media` for every
/// ROM-bearing variant.
pub(super) fn mpi_slot_from_cart(cart: &Cart, i: u8, media: &MediaRefs) -> MPISlot {
    let rom_path = || {
        media
            .cart_roms
            .iter()
            .find(|r| r.mpi_slot == Some(i))
            .map(|r| r.rom.path.clone())
    };
    match cart {
        Cart::ROMPak(_) => rom_path().map(MPISlot::ROMPak).unwrap_or(MPISlot::Empty),
        Cart::BankedROMPak(_) => rom_path()
            .map(MPISlot::BankedROMPak)
            .unwrap_or(MPISlot::Empty),
        Cart::GamesMasterCartridge(_) => rom_path()
            .map(MPISlot::GamesMasterCartridge)
            .unwrap_or(MPISlot::Empty),
        Cart::Orch90(_) => rom_path().map(MPISlot::Orch90).unwrap_or(MPISlot::Empty),
        Cart::DiskCart(_) => MPISlot::FD502,
        Cart::DistoRTC(_) => MPISlot::DistoRTC,
        Cart::DeluxeRS232(_) => MPISlot::DeluxeRS232(rom_path()),
        Cart::SoundSpeechCartridge(_) => MPISlot::SoundSpeechCartridge,
        _ => MPISlot::Empty,
    }
}

/// Hash the file at `path` into a [`MediaRef`] (SAVE side).
pub(super) fn hash_media_ref(path: &Path) -> Result<MediaRef, String> {
    let sha256 = snapshot::sha256_file(path)
        .map_err(|e| format!("could not hash {}: {e}", path.display()))?;
    Ok(MediaRef {
        path: path.to_path_buf(),
        sha256,
    })
}

/// LOAD side: `mr`'s bytes if its file is present (pushing a `warnings`
/// entry first on hash mismatch), or `None` if it's missing.
pub(super) fn read_if_present(
    mr: &MediaRef,
    role: &str,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    match mr.verify() {
        MediaCheck::Missing => None,
        MediaCheck::Mismatch { .. } => {
            warnings.push(mismatch_warning(role, mr));
            std::fs::read(&mr.path).ok()
        }
        MediaCheck::Ok => std::fs::read(&mr.path).ok(),
    }
}

/// [`read_if_present`]'s sibling for VHD/DriveWire sources, which need a
/// read-write file handle rather than bytes.
pub(super) fn open_if_present(
    mr: &MediaRef,
    role: &str,
    warnings: &mut Vec<String>,
) -> Option<std::fs::File> {
    match mr.verify() {
        MediaCheck::Missing => None,
        MediaCheck::Mismatch { .. } => {
            warnings.push(mismatch_warning(role, mr));
            open_read_write(&mr.path).ok()
        }
        MediaCheck::Ok => open_read_write(&mr.path).ok(),
    }
}

/// Mirrors [`crate::CocoApp::insert_vhd`]/[`crate::CocoApp::insert_dw_disk`]'s
/// own `OpenOptions` — VHD/DriveWire writes hit the backing file directly.
fn open_read_write(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

fn mismatch_warning(role: &str, mr: &MediaRef) -> String {
    format!(
        "{role} ({}) doesn't match the hash recorded in this save state; loaded anyway",
        mr.path.display()
    )
}

/// True if `path` is a [`crate::ROMSource::ComposedCoco12`] pseudo-path
/// ([`ROM_DB_PSEUDO_PATH_PREFIX`]) rather than a real filesystem path.
pub(super) fn is_rom_db_pseudo_path(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|s| s.starts_with(ROM_DB_PSEUDO_PATH_PREFIX))
}

/// [`coco_core::snapshot::MediaSources::system_rom`] resolver: a real path
/// reads normally; a [`ROM_DB_PSEUDO_PATH_PREFIX`] pseudo-path recomposes
/// from [`installed_roms_dir`] instead — "missing" only means no local BASIC dump exists.
pub(super) fn resolve_system_rom(
    media: &MediaRefs,
    warnings: &mut Vec<String>,
) -> Option<Box<[u8]>> {
    let mr = media.system_rom.as_ref()?;
    if is_rom_db_pseudo_path(&mr.path) {
        return match compose_coco12_rom(&installed_roms_dir()) {
            Coco12ROMResult::Composed { image, .. } => {
                let actual = snapshot::sha256_hex(&image);
                if actual != mr.sha256 {
                    warnings.push(format!(
                        "system ROM ({}): local Color/Extended BASIC dumps differ from the ones \
                         this state was saved with; loaded anyway",
                        mr.path.display()
                    ));
                }
                Some(image)
            }
            Coco12ROMResult::NoColorBasic => None,
        };
    }
    read_if_present(mr, "system ROM", warnings).map(Vec::into_boxed_slice)
}

/// [`coco_core::snapshot::MediaSources::cart_roms`] resolver. `ssc_slots`
/// names every slot holding a Sound/Speech Cartridge: a snapshot from
/// before the firmware was emulated recorded only its SP0256 ROM, so the
/// installed firmware image stands in, with a warning.
pub(super) fn resolve_cart_roms(
    media: &MediaRefs,
    ssc_slots: &[Option<u8>],
    warnings: &mut Vec<String>,
) -> Vec<CartROMSource> {
    let mut out: Vec<CartROMSource> = media
        .cart_roms
        .iter()
        .filter_map(|slot_ref| {
            read_if_present(&slot_ref.rom, "cartridge ROM", warnings).map(|bytes| CartROMSource {
                mpi_slot: slot_ref.mpi_slot,
                role: slot_ref.role,
                bytes,
            })
        })
        .collect();
    for &slot in ssc_slots {
        let recorded = media
            .cart_roms
            .iter()
            .any(|r| r.mpi_slot == slot && r.role == CartROMRole::SSCFirmware);
        if recorded {
            continue;
        }
        let path = crate::rom_load::ssc_firmware_rom_path();
        match std::fs::read(&path) {
            Ok(bytes) => {
                warnings.push(format!(
                    "save state predates the SSC firmware reference; using {}",
                    path.display()
                ));
                out.push(CartROMSource {
                    mpi_slot: slot,
                    role: CartROMRole::SSCFirmware,
                    bytes,
                });
            }
            Err(e) => warnings.push(format!(
                "save state predates the SSC firmware reference and {} cannot be read: {e}",
                path.display()
            )),
        }
    }
    out
}
