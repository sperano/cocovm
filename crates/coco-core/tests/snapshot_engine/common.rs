use std::path::PathBuf;

use coco_core::snapshot::{self, MediaRef, MediaRefs, SnapshotError};
use coco_core::{Machine, MachineConfig};
use test_assets::rom::COCO3;

pub fn rom_path() -> PathBuf {
    test_assets::rom(COCO3)
}

pub fn load_rom() -> Box<[u8]> {
    std::fs::read(rom_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", rom_path().display()))
        .into_boxed_slice()
}

pub fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

/// `Result::unwrap_err` requires `T: Debug`, which `SnapshotPayload` and
/// `RestoredMachine` deliberately do not implement. They carry a whole
/// `Machine`, including RAM, so a `Debug` dump is not useful. Match the result
/// instead, and name this helper for clarity at each call site.
pub fn expect_err<T>(result: Result<T, SnapshotError>) -> SnapshotError {
    match result {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e,
    }
}

/// A `MediaRefs` recording only the real system ROM, for tests that need a
/// save/restore round trip but no other media.
pub fn system_rom_only_media() -> MediaRefs {
    MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        ..MediaRefs::default()
    }
}
