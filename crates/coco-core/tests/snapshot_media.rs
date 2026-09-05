//! Save-state engine coverage for the floppy-specific missing-media and
//! wrong-shape restore paths that `tests/snapshot_engine.rs`'s phase-2 suite
//! doesn't reach (its `MissingMedia`/`MediaShape` tests only exercise
//! cart-ROM images, never a mounted `JvcDisk`) — phase 3 spec item 4.
//! These are engine-level tests: they call `save`, `load`, and `restore` without
//! stepping the CPU.

use std::path::PathBuf;

use coco_core::fdc::{DiskCart, JVCDisk};
use coco_core::snapshot::{
    self, CartROMRole, CartROMSource, MediaRef, MediaRefs, MediaSources, SlotROMRef, SnapshotError,
};
use coco_core::{Machine, MachineConfig};
use test_assets::rom::{COCO3, DISK11};

fn rom_path() -> PathBuf {
    test_assets::rom(COCO3)
}

fn load_rom() -> Box<[u8]> {
    std::fs::read(rom_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", rom_path().display()))
        .into_boxed_slice()
}

fn disk_rom_path() -> PathBuf {
    test_assets::rom(DISK11)
}

fn load_disk_rom() -> Box<[u8]> {
    std::fs::read(disk_rom_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", disk_rom_path().display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

/// One headerless track (18 sectors x 256B).
const ONE_TRACK_BYTES: usize = 18 * 256;

/// `Result::unwrap_err` requires `T: Debug`, which `RestoredMachine` does not
/// implement. Match the result instead, as `snapshot_engine.rs` does, because
/// the restored machine is not useful in a debug dump.
fn expect_err<T>(result: Result<T, SnapshotError>) -> SnapshotError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e,
    }
}

/// A `MediaRefs` recording the real system ROM plus the disk controller's
/// own ROM, for tests that only care about the *disk* media path.
fn media_with_disk_rom(disk_bytes: &[u8], disk_path: &str) -> MediaRefs {
    MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        cart_roms: vec![SlotROMRef {
            mpi_slot: None,
            role: CartROMRole::Primary,
            rom: MediaRef {
                path: disk_rom_path(),
                sha256: snapshot::sha256_hex(&load_disk_rom()),
            },
        }],
        disks: vec![
            Some(MediaRef {
                path: PathBuf::from(disk_path),
                sha256: snapshot::sha256_hex(disk_bytes),
            }),
            None,
            None,
            None,
        ],
        ..MediaRefs::default()
    }
}

#[test]
fn missing_disk_source_is_missing_media_naming_the_disk_and_its_path() {
    let mut machine = boot_machine();
    let mut cart = DiskCart::new(load_disk_rom());
    let disk_bytes = vec![0u8; ONE_TRACK_BYTES];
    cart.insert_disk(
        0,
        JVCDisk::from_bytes(disk_bytes.clone()).expect("build disk"),
    );
    machine.insert_cartridge(cart);

    let disk_path = "my-floppy.jvc";
    let media = media_with_disk_rom(&disk_bytes, disk_path);
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");

    // Every source resolved except the disk itself.
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        cart_roms: vec![CartROMSource::primary(None, load_disk_rom().to_vec())],
        ..MediaSources::default()
    };
    let err = expect_err(snapshot::restore(payload, sources));
    match err {
        SnapshotError::MissingMedia { descriptions } => {
            assert!(
                descriptions
                    .iter()
                    .any(|d| d.contains("floppy") && d.contains(disk_path)),
                "descriptions {descriptions:?} should name the floppy and its path {disk_path:?}"
            );
        }
        other => panic!("expected MissingMedia, got {other:?}"),
    }
}

#[test]
fn reattached_disk_with_different_geometry_is_a_media_shape_error() {
    let mut machine = boot_machine();
    let mut cart = DiskCart::new(load_disk_rom());
    // 35-track headerless image (default geometry).
    let original_bytes = vec![0u8; 35 * ONE_TRACK_BYTES];
    cart.insert_disk(
        0,
        JVCDisk::from_bytes(original_bytes.clone()).expect("build disk"),
    );
    machine.insert_cartridge(cart);

    let media = media_with_disk_rom(&original_bytes, "shifted.jvc");
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");

    // A valid headerless image, but with a different track count than the
    // snapshot recorded — the file was reformatted, truncated, or grown since.
    let reshaped_bytes = vec![0u8; 10 * ONE_TRACK_BYTES];
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        cart_roms: vec![CartROMSource::primary(None, load_disk_rom().to_vec())],
        disks: [Some(reshaped_bytes), None, None, None],
        ..MediaSources::default()
    };
    let err = expect_err(snapshot::restore(payload, sources));
    match err {
        SnapshotError::MediaShape { role, detail } => {
            assert!(
                role.contains("floppy") && role.contains("drive 0"),
                "role: {role:?}"
            );
            assert!(detail.contains("geometry"), "detail: {detail:?}");
        }
        other => panic!("expected MediaShape, got {other:?}"),
    }
}
