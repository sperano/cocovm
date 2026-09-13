//! `CocoApp::new`'s DriveWire launch path — the [`DriveWireLaunch`] half of
//! [`AppParams`] that no production caller builds yet (`machine_def.rs` has
//! no DriveWire fields), exercised here so the becker-mount branch stays
//! covered.

use super::*;
use crate::rom_load::COCO3_ROM_FILE;

/// Asset-free: the default params must leave DriveWire off — the invariant
/// every production launch and every other test relies on.
#[test]
fn appparams_default_leaves_drivewire_off() {
    assert!(
        AppParams::default().drivewire.is_none(),
        "AppParams::default() must leave DriveWire off"
    );
}

/// Scratch space under `target/` (git-ignored).
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-app")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Boots using the installed `coco3.rom`.
fn boot_with_drivewire(drivewire: Option<DriveWireLaunch>) -> CocoApp {
    let rom_path = installed_roms_dir().join(COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams {
            drivewire,
            ..AppParams::default()
        },
        crate::joy::SharedGamepad::without_backend(),
    )
}

/// `Some(DriveWireLaunch)` must enable the Becker port with the requested
/// HDB-DOS mode, for both values (so a hardcoded `enable_drivewire(true)` can't pass).
#[test]
fn drivewire_launch_enables_becker_with_hdbdos_mode() {
    for hdbdos_mode in [false, true] {
        let app = boot_with_drivewire(Some(DriveWireLaunch {
            hdbdos_mode,
            ..DriveWireLaunch::default()
        }));
        let dw = app
            .machine
            .bus
            .drivewire
            .as_ref()
            .expect("DriveWireLaunch must enable the Becker port");
        assert_eq!(
            dw.hdbdos_mode(),
            hdbdos_mode,
            "hdbdos_mode: {hdbdos_mode} must reach set_hdbdos_mode"
        );
    }
}

/// A `disk_paths` slot must reach `insert_dw_disk`: the image mounts with no
/// error and the path is tracked in `dw_paths`.
#[test]
fn drivewire_launch_mounts_disk_images() {
    let path = scratch_dir("dw-mount").join("dw0.dsk");
    std::fs::write(&path, vec![0u8; 256]).expect("write DW image fixture");

    let mut disk_paths: [Option<PathBuf>; drivewire::DRIVE_COUNT] = std::array::from_fn(|_| None);
    disk_paths[0] = Some(path.clone());
    let app = boot_with_drivewire(Some(DriveWireLaunch {
        disk_paths,
        ..DriveWireLaunch::default()
    }));

    assert!(
        app.cart_error.is_none(),
        "mounting the DW image: {:?}",
        app.cart_error
    );
    assert_eq!(app.dw_paths[0], Some(path), "drive 0 path must be tracked");
}
