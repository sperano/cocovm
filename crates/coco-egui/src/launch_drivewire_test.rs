use std::path::Path;

use coco_core::MachineConfig;

use crate::machine_def::{CartridgeDTO, DriveWireDTO, MachineDef, SlotDTO};

fn base_def() -> MachineDef {
    MachineDef::from_config(
        "DriveWire Test".to_string(),
        None,
        &MachineConfig::default(),
    )
}

fn drivewire_def(path: &Path, hdbdos_mode: bool) -> MachineDef {
    let mut def = base_def();
    def.drivewire = DriveWireDTO {
        enabled: true,
        hdbdos_mode,
        disk0: Some(path.display().to_string()),
        ..DriveWireDTO::default()
    };
    def
}

#[test]
fn relative_paths_resolve_for_all_drives() {
    let mut def = base_def();
    def.drivewire.disk0 = Some("zero.dsk".to_string());
    def.drivewire.disk1 = Some("one.dsk".to_string());
    def.drivewire.disk2 = Some("two.dsk".to_string());
    def.drivewire.disk3 = Some("three.dsk".to_string());

    let media = super::resolve_media(&def, "relative-drivewire");
    for (drive, name) in ["zero.dsk", "one.dsk", "two.dsk", "three.dsk"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            media.drivewire[drive],
            Some(crate::machine_def::resolve_media_path(
                name,
                "relative-drivewire"
            ))
        );
    }
}

#[test]
fn definition_enables_mode_and_mounts_media() {
    let dir = crate::machine_def::tests::TempDir::new("launch-drivewire");
    let image = dir.path().join("dw0.dsk");
    std::fs::write(&image, vec![0_u8; coco_core::drivewire::SECTOR_SIZE])
        .expect("write DriveWire fixture");

    for hdbdos_mode in [false, true] {
        let app = super::launch_machine(&drivewire_def(&image, hdbdos_mode), "drivewire")
            .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
        let server = app.machine.bus.drivewire.as_ref().expect("Becker enabled");
        assert_eq!(server.hdbdos_mode(), hdbdos_mode);
        assert!(server.is_mounted(0));
        assert_eq!(app.dw_paths[0].as_deref(), Some(image.as_path()));
    }
}

#[test]
fn launched_vms_keep_independent_drivewire_state() {
    let dir = crate::machine_def::tests::TempDir::new("launch-drivewire-isolation");
    let first = dir.path().join("first.dsk");
    let second = dir.path().join("second.dsk");
    std::fs::write(&first, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).unwrap();
    std::fs::write(&second, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).unwrap();

    let first_app = super::launch_machine(&drivewire_def(&first, false), "dw-first").unwrap();
    let second_app = super::launch_machine(&drivewire_def(&second, true), "dw-second").unwrap();
    assert_eq!(first_app.dw_paths[0].as_deref(), Some(first.as_path()));
    assert_eq!(second_app.dw_paths[0].as_deref(), Some(second.as_path()));
    assert!(
        !first_app
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .hdbdos_mode()
    );
    assert!(
        second_app
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .hdbdos_mode()
    );
}

#[test]
fn disabled_drivewire_ignores_configured_missing_media() {
    let mut def = drivewire_def(Path::new("/missing/dw0.dsk"), false);
    def.drivewire.enabled = false;
    let app = super::launch_machine(&def, "drivewire-disabled").unwrap();
    assert!(app.machine.bus.drivewire.is_none());
}

#[test]
fn enabled_drivewire_rejects_missing_or_non_file_media() {
    let dir = crate::machine_def::tests::TempDir::new("launch-drivewire-invalid");
    for path in [dir.path().join("missing.dsk"), dir.path().to_path_buf()] {
        let err = super::launch_machine(&drivewire_def(&path, false), "drivewire-invalid")
            .err()
            .expect("invalid DriveWire media should fail launch");
        assert!(err.contains("DriveWire image"), "unexpected error: {err}");
    }
}

#[cfg(unix)]
#[test]
fn enabled_drivewire_rejects_read_only_media() {
    let dir = crate::machine_def::tests::TempDir::new("launch-drivewire-read-only");
    let image = dir.path().join("read-only.dsk");
    std::fs::write(&image, vec![0_u8; coco_core::drivewire::SECTOR_SIZE]).unwrap();
    let _read_only = crate::save_state::tests::ReadOnly::new(&image);

    let err = super::launch_machine(&drivewire_def(&image, false), "drivewire-read-only")
        .err()
        .expect("read-only DriveWire media should fail launch");
    assert!(err.contains("could not open"), "unexpected error: {err}");
    assert!(err.contains("read-only.dsk"), "unexpected error: {err}");
}

#[test]
fn enabled_drivewire_rejects_games_master_direct_or_in_mpi() {
    let direct = CartridgeDTO::GamesMaster {
        path: "game.rom".to_string(),
    };
    let mpi = CartridgeDTO::MPI {
        slots: [
            SlotDTO::GamesMaster {
                path: "game.rom".to_string(),
            },
            SlotDTO::Empty,
            SlotDTO::Empty,
            SlotDTO::Empty,
        ],
        switch: 1,
    };
    for cartridge in [direct, mpi] {
        let mut def = base_def();
        def.drivewire.enabled = true;
        def.peripherals.cartridge = cartridge;
        let err = super::launch_machine(&def, "drivewire-conflict")
            .err()
            .expect("Games Master and DriveWire should conflict");
        assert!(err.contains("$FF41"), "unexpected error: {err}");
    }
}
