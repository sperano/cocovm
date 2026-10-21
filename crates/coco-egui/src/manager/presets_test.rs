use std::fs;

use coco_core::{MachineVariant, MemorySize, MonitorType, VDGVariant};

use super::*;
use crate::machine_def::tests::TempDir;
use crate::manager::{ManagerApp, NO_DATA_DIR};

const CREATED: &str = "2026-10-21";

fn created() -> Option<String> {
    Some(CREATED.to_string())
}

/// Every template converts to a config the core accepts, and its
/// DriveWire/cartridge combination passes the same checks a save does.
#[test]
fn every_preset_validates() {
    for preset in Preset::ALL {
        preset
            .def(created())
            .to_machine_config()
            .unwrap_or_else(|e| panic!("{preset:?} must validate: {e}"));
    }
}

/// Each template has the model, RAM, display, VDG, and DOS ROM its
/// `description` claims.
#[test]
fn presets_have_the_promised_hardware() {
    let coco3 = Preset::CoCo3DiskBasic
        .def(None)
        .to_machine_config()
        .unwrap();
    assert_eq!(coco3.variant, MachineVariant::Coco3);
    assert_eq!(coco3.memory, MemorySize::K512);
    assert_eq!(coco3.monitor, Some(MonitorType::RGB));

    let coco2 = Preset::CoCo2DiskBasic
        .def(None)
        .to_machine_config()
        .unwrap();
    assert_eq!(coco2.variant, MachineVariant::Coco2);
    assert_eq!(coco2.memory, MemorySize::K64);
    assert_eq!(coco2.vdg, Some(VDGVariant::MC6847T1));

    for preset in [Preset::CoCo3DiskBasic, Preset::CoCo2DiskBasic] {
        let def = preset.def(None);
        assert_eq!(def.peripherals.cartridge.dos_rom(), Some(DosRom::DiskBasic));
        assert_eq!(def.media.disk0.as_deref(), Some("disk0.dsk"));
        assert!(!def.drivewire.enabled);
    }

    let drivewire = Preset::CoCo3DriveWire.def(None);
    assert!(drivewire.peripherals.cartridge.uses_hdbdos());
    assert!(drivewire.drivewire.enabled);
    assert!(drivewire.drivewire.hdbdos_mode);
    assert_eq!(drivewire.media.disk0, None);
}

/// A template's definition is named after the template and dated as given.
#[test]
fn preset_def_takes_its_name_and_date() {
    let def = Preset::CoCo3DriveWire.def(created());
    assert_eq!(def.name, "CoCo 3 DriveWire (HDB-DOS)");
    assert_eq!(def.created.as_deref(), Some(CREATED));
}

/// `install` writes a definition that loads back unchanged, plus the blank
/// floppy its `[media].disk0` names.
#[test]
fn install_round_trips_and_places_the_blank_disk() {
    for preset in Preset::ALL {
        let machines = TempDir::new("preset-install-machines");
        let artifacts = TempDir::new("preset-install-artifacts");
        let slug = machine_def::slugify(preset.name());

        let def = install(
            preset,
            machines.path(),
            Some(artifacts.path()),
            &slug,
            created(),
        )
        .unwrap_or_else(|e| panic!("{preset:?} must install: {e}"));

        let loaded = machine_def::load_all(machines.path()).expect("installed preset loads");
        assert_eq!(loaded, vec![(slug.clone(), def.clone())]);
        let disk = artifacts.path().join(&slug).join("disk0.dsk");
        assert_eq!(disk.is_file(), def.media.disk0.is_some(), "{preset:?}");
    }
}

/// A template with a blank disk needs the artifact root; without one it
/// fails before writing a definition that would point at a missing image.
#[test]
fn install_without_artifact_root_writes_nothing() {
    let machines = TempDir::new("preset-no-artifacts");
    let error = install(
        Preset::CoCo3DiskBasic,
        machines.path(),
        None,
        "coco-3-disk-basic",
        None,
    )
    .unwrap_err();
    assert_eq!(error, NO_DATA_DIR);
    assert!(!machine_def::def_path(machines.path(), "coco-3-disk-basic").exists());
}

/// Picking the same template twice in the manager makes two independent
/// machines under distinct slugs, each with its own blank floppy.
#[test]
fn creating_a_preset_twice_gives_each_copy_its_own_slug() {
    let machines = TempDir::new("preset-twice-machines");
    let artifacts = TempDir::new("preset-twice-artifacts");
    let mut app = ManagerApp::new(
        None,
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
        None,
    );

    app.create_machine_from_preset(Preset::CoCo3DiskBasic);
    app.create_machine_from_preset(Preset::CoCo3DiskBasic);

    assert_eq!(app.save_error, None);
    let mut slugs: Vec<&str> = app.entries.iter().map(|e| e.slug.as_str()).collect();
    slugs.sort_unstable();
    assert_eq!(slugs, ["coco-3-disk-basic", "coco-3-disk-basic-2"]);
    for slug in slugs {
        assert!(artifacts.path().join(slug).join("disk0.dsk").is_file());
    }
}

/// A missing machines directory is a first run: the starter templates are
/// written under their own slugs.
#[test]
fn first_run_seeds_the_starter_machines() {
    let config = TempDir::new("preset-seed-config");
    let artifacts = TempDir::new("preset-seed-artifacts");
    let machines = config.path().join("machines");

    seed_first_run(&machines, Some(artifacts.path()), created()).expect("seeding succeeds");

    let loaded = machine_def::load_all(&machines).expect("seeded machines load");
    let slugs: Vec<&str> = loaded.iter().map(|(slug, _)| slug.as_str()).collect();
    assert_eq!(slugs, ["coco-2-disk-basic", "coco-3-disk-basic"]);
    for (slug, def) in &loaded {
        assert_eq!(def.created.as_deref(), Some(CREATED));
        assert!(artifacts.path().join(slug).join("disk0.dsk").is_file());
    }
}

/// An existing machines directory is never seeded, even when empty: the
/// user may have deleted every machine on purpose.
#[test]
fn existing_machines_dir_is_not_seeded() {
    let machines = TempDir::new("preset-seed-existing");
    let artifacts = TempDir::new("preset-seed-existing-artifacts");

    seed_first_run(machines.path(), Some(artifacts.path()), created()).expect("no-op");

    assert_eq!(fs::read_dir(machines.path()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(artifacts.path()).unwrap().count(), 0);
}

/// Each template boots through the production launch path with the
/// installed ROMs: its DOS ROM loads and its blank floppy mounts. The
/// relative `disk0` is pointed at the placed file, since launch resolves it
/// against the real data directory.
#[test]
fn every_preset_launches() {
    for preset in Preset::ALL {
        let machines = TempDir::new("preset-launch-machines");
        let artifacts = TempDir::new("preset-launch-artifacts");
        let slug = machine_def::slugify(preset.name());
        let mut def = install(preset, machines.path(), Some(artifacts.path()), &slug, None)
            .unwrap_or_else(|e| panic!("{preset:?} must install: {e}"));
        if let Some(disk) = &mut def.media.disk0 {
            *disk = artifacts
                .path()
                .join(&slug)
                .join(&*disk)
                .display()
                .to_string();
        }

        let mut app = crate::launch::launch_machine(&def, &slug)
            .unwrap_or_else(|e| panic!("{preset:?} must launch: {e}"));
        assert!(
            app.machine.bus.cart.as_disk_cart().is_some(),
            "{preset:?} must have its FD-502 installed"
        );
        assert_eq!(
            app.disk_paths[BLANK_DISK_DRIVE].is_some(),
            def.media.disk0.is_some(),
            "{preset:?}: blank disk mount"
        );
    }
}
