use super::*;

use coco_core::MachineConfig;
use coco_core::snapshot::{CartROMRole, SlotROMRef};

fn media_ref(path: PathBuf) -> MediaRef {
    MediaRef {
        path,
        sha256: "hash".to_string(),
    }
}

#[test]
fn definition_remap_changes_only_absolute_managed_paths() {
    let old_dir = Path::new("/data/machines/old");
    let new_dir = Path::new("/data/machines/new");
    let external = "/media/external/game.rom";
    let mut def = MachineDef::from_config("Name".to_string(), None, &MachineConfig::default());
    def.hardware.rom = Some(old_dir.join("system.rom").to_string_lossy().into_owned());
    def.media.disk0 = Some("disk0.dsk".to_string());
    def.media.disk1 = Some(external.to_string());
    def.drivewire.disk0 = Some(old_dir.join("dw0.dsk").to_string_lossy().into_owned());
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::ROMPak {
                path: old_dir.join("pak.rom").to_string_lossy().into_owned(),
            },
            SlotDTO::default(),
            SlotDTO::default(),
            SlotDTO::default(),
        ],
        switch: 1,
    };

    remap_definition_paths(&mut def, old_dir, new_dir);

    assert_eq!(
        def.hardware.rom.as_deref(),
        Some("/data/machines/new/system.rom")
    );
    assert_eq!(def.media.disk0.as_deref(), Some("disk0.dsk"));
    assert_eq!(def.media.disk1.as_deref(), Some(external));
    assert_eq!(
        def.drivewire.disk0.as_deref(),
        Some("/data/machines/new/dw0.dsk")
    );
    let CartridgeDTO::MPI { slots, .. } = &def.peripherals.cartridge else {
        panic!("MPI fixture must remain an MPI");
    };
    let SlotDTO::ROMPak { path, .. } = &slots[0] else {
        panic!("ROM Pak fixture must remain a ROM Pak");
    };
    assert_eq!(path, "/data/machines/new/pak.rom");
}

#[test]
fn snapshot_remap_changes_every_managed_reference() {
    let old_dir = Path::new("/data/machines/old");
    let new_dir = Path::new("/data/machines/new");
    let external = PathBuf::from("/media/external/disk.dsk");
    let mut refs = MediaRefs {
        system_rom: Some(media_ref(old_dir.join("system.rom"))),
        cart_roms: vec![SlotROMRef {
            mpi_slot: None,
            role: CartROMRole::Primary,
            rom: media_ref(old_dir.join("pak.rom")),
        }],
        disks: vec![
            Some(media_ref(old_dir.join("disk0.dsk"))),
            Some(media_ref(external.clone())),
        ],
        vhds: vec![Some(media_ref(old_dir.join("hd0.vhd")))],
        drivewire: vec![Some(media_ref(old_dir.join("dw0.dsk")))],
        tape: Some(media_ref(old_dir.join("tape.cas"))),
    };

    remap_media_refs(&mut refs, old_dir, new_dir);

    assert_eq!(refs.system_rom.unwrap().path, new_dir.join("system.rom"));
    assert_eq!(refs.cart_roms[0].rom.path, new_dir.join("pak.rom"));
    assert_eq!(
        refs.disks[0].as_ref().unwrap().path,
        new_dir.join("disk0.dsk")
    );
    assert_eq!(refs.disks[1].as_ref().unwrap().path, external);
    assert_eq!(refs.vhds[0].as_ref().unwrap().path, new_dir.join("hd0.vhd"));
    assert_eq!(
        refs.drivewire[0].as_ref().unwrap().path,
        new_dir.join("dw0.dsk")
    );
    assert_eq!(refs.tape.unwrap().path, new_dir.join("tape.cas"));
}
