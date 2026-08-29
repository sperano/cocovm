//! 3. Missing system ROM source
//! 4. `MediaRef::verify`
//! 5. No-ROM-bytes gate (structural), plus cart-ROM reattachment. The
//!    reattachment test exercises `restore_cart_roms` and `require_cart_rom`.
//!    Other tests either have no cartridge or never call `restore`.
//! 6. RAM-length tamper

use std::path::PathBuf;

use coco_core::cart::{Cart, Cartridge, ROMPak};
use coco_core::snapshot::{
    self, MediaCheck, MediaRef, MediaRefs, MediaSources, SlotROMRef, SnapshotError, SnapshotPayload,
};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};

use super::common::{boot_machine, expect_err, load_rom, rom_path, system_rom_only_media};

#[test]
fn missing_system_rom_source_is_missing_media() {
    let original = boot_machine();
    let media = system_rom_only_media();
    let bytes = snapshot::save(&original, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");

    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    match err {
        SnapshotError::MissingMedia { descriptions } => {
            assert!(
                descriptions.iter().any(|d| d.contains("system ROM")),
                "descriptions {descriptions:?} should mention the system ROM"
            );
        }
        other => panic!("expected MissingMedia, got {other:?}"),
    }
}

/// Returns a scratch file path under the OS temp directory, unique to this
/// test process. The caller removes the file after the test.
fn scratch_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "coco_snapshot_engine_test_{}_{name}",
        std::process::id()
    ))
}

#[test]
fn media_ref_verify_reports_ok_mismatch_and_missing() {
    let path = scratch_path("verify");
    std::fs::write(&path, b"hello coco").unwrap();
    let media_ref = MediaRef {
        path: path.clone(),
        sha256: snapshot::sha256_hex(b"hello coco"),
    };
    assert_eq!(media_ref.verify(), MediaCheck::Ok);

    std::fs::write(&path, b"goodbye coco").unwrap();
    match media_ref.verify() {
        MediaCheck::Mismatch { actual } => {
            assert_eq!(actual, snapshot::sha256_hex(b"goodbye coco"));
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }

    std::fs::remove_file(&path).unwrap();
    assert_eq!(media_ref.verify(), MediaCheck::Missing);
}

#[test]
fn restored_payload_carries_no_rom_bytes_before_reattachment() {
    let mut machine = boot_machine();
    let pak_image = vec![0xA5u8; 4096];
    machine.insert_cartridge(ROMPak::from_bytes(&pak_image, false).expect("build pak"));

    let bytes = snapshot::save(&machine, &MediaRefs::default()).expect("save");
    let payload: SnapshotPayload = snapshot::load(&bytes).expect("load");

    assert!(
        payload.machine.bus.rom.is_empty(),
        "deserialized-but-not-yet-reattached system ROM must be empty"
    );
    let cart_debug = format!("{:?}", payload.machine.bus.cart);
    assert!(
        matches!(payload.machine.bus.cart, Cart::ROMPak(_)),
        "expected a ROMPak cart, got {cart_debug}"
    );
    assert!(
        cart_debug.contains("image_len: 0"),
        "deserialized-but-not-yet-reattached pak image must be empty, got {cart_debug}"
    );
}

#[test]
fn direct_port_cart_rom_is_reattached_through_a_full_restore() {
    let mut machine = boot_machine();
    // A uniform fill lets any byte read back prove that the *mirrored* image
    // (rather than only offset 0) survived reattachment, without needing to work
    // out `ROMPak`'s half-swap indexing by hand.
    let pak_image = vec![0x42u8; 1024];
    machine.insert_cartridge(ROMPak::from_bytes(&pak_image, false).expect("build pak"));

    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        cart_roms: vec![SlotROMRef {
            mpi_slot: None,
            rom: MediaRef {
                path: PathBuf::from("pak.rom"),
                sha256: snapshot::sha256_hex(&pak_image),
            },
        }],
        ..MediaRefs::default()
    };
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        cart_roms: vec![(None, pak_image)],
        ..MediaSources::default()
    };
    let restored = snapshot::restore(payload, sources).expect("restore");

    match restored.machine.bus.cart {
        Cart::ROMPak(pak) => assert_eq!(pak.rom_peek(0x8000), 0x42),
        other => panic!("expected ROMPak, got {other:?}"),
    }
}

#[test]
fn missing_cart_rom_source_is_missing_media() {
    let mut machine = boot_machine();
    let pak_image = vec![0x99u8; 1024];
    machine.insert_cartridge(ROMPak::from_bytes(&pak_image, false).expect("build pak"));

    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path(),
            sha256: snapshot::sha256_file(&rom_path()).expect("hash roms/coco3.rom"),
        }),
        cart_roms: vec![SlotROMRef {
            mpi_slot: None,
            rom: MediaRef {
                path: PathBuf::from("pak.rom"),
                sha256: snapshot::sha256_hex(&pak_image),
            },
        }],
        ..MediaRefs::default()
    };
    let bytes = snapshot::save(&machine, &media).expect("save");
    let payload = snapshot::load(&bytes).expect("load");
    // No `cart_roms` entry in `sources`: the pak's image is unresolved.
    let sources = MediaSources {
        system_rom: Some(load_rom()),
        ..MediaSources::default()
    };

    let err = expect_err(snapshot::restore(payload, sources));
    match err {
        SnapshotError::MissingMedia { descriptions } => {
            assert!(
                descriptions.iter().any(|d| d.contains("ROMPak")),
                "descriptions {descriptions:?} should mention the missing ROMPak ROM"
            );
        }
        other => panic!("expected MissingMedia, got {other:?}"),
    }
}

#[test]
fn ram_length_mismatch_is_an_invalid_payload_error_not_a_panic() {
    let config = MachineConfig {
        variant: MachineVariant::Coco3,
        memory: MemorySize::K128,
        ..MachineConfig::default()
    };
    let mut machine = Machine::new(config, Box::new([]));
    // Hand-tamper the RAM length so it no longer matches `config.memory`.
    machine.bus.ram = vec![0u8; 1].into_boxed_slice();

    let payload = SnapshotPayload {
        media: MediaRefs::default(),
        machine,
    };
    let err = expect_err(snapshot::restore(payload, MediaSources::default()));
    assert!(matches!(err, SnapshotError::InvalidPayload(_)), "{err:?}");
}

/// Cheap companion to #5/#6: confirms the streamed [`snapshot::sha256_file`]
/// agrees with the in-memory [`snapshot::sha256_hex`] on the same bytes.
#[test]
fn sha256_file_matches_sha256_hex_of_the_same_bytes() {
    let path = scratch_path("hash");
    let content = b"the quick brown fox jumps over the lazy dog";
    std::fs::write(&path, content).unwrap();
    assert_eq!(
        snapshot::sha256_file(&path).unwrap(),
        snapshot::sha256_hex(content)
    );
    std::fs::remove_file(&path).unwrap();
}
