use std::fs;
use std::sync::Barrier;
use std::thread;

use coco_core::{MachineVariant, MemorySize, MonitorType, VDGVariant};

use super::*;
use crate::machine_def::tests::TempDir;
use crate::manager::{ManagerApp, NO_DATA_DIR};

const CREATED: &str = "2026-10-21";

fn created() -> Option<String> {
    Some(CREATED.to_string())
}

/// Stand-in bytes for the downloaded EOU images.
const FAKE_EOU_IMAGE: &[u8] = b"eou image";

/// A directory holding stand-ins for both downloaded EOU images.
fn eou_fixture() -> TempDir {
    let dir = TempDir::new("preset-eou-images");
    for name in EOU_IMAGES {
        fs::write(dir.path().join(name), FAKE_EOU_IMAGE).unwrap();
    }
    dir
}

/// [`install`]'s directories.
fn dirs<'a>(
    machines_dir: &'a Path,
    artifacts_root: Option<&'a Path>,
    eou_dir: Option<&'a Path>,
) -> InstallDirs<'a> {
    InstallDirs {
        machines_dir,
        artifacts_root,
        eou_dir,
    }
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
    let eou = eou_fixture();
    for preset in Preset::ALL {
        let machines = TempDir::new("preset-install-machines");
        let artifacts = TempDir::new("preset-install-artifacts");
        let slug = machine_def::slugify(preset.name());

        let def = install(
            preset,
            &dirs(machines.path(), Some(artifacts.path()), Some(eou.path())),
            &slug,
            created(),
        )
        .unwrap_or_else(|e| panic!("{preset:?} must install: {e}"));

        let loaded = machine_def::load_all(machines.path()).expect("installed preset loads");
        assert_eq!(loaded, vec![(slug.clone(), def.clone())]);
        // Without a drive 0 image, no blank floppy may be written either.
        let blank = blank_disk_file(BLANK_DISK_DRIVE);
        let disk = artifacts
            .path()
            .join(&slug)
            .join(def.media.disk0.as_deref().unwrap_or(&blank));
        assert_eq!(
            disk.is_file(),
            def.media.disk0.is_some(),
            "{preset:?}: {}",
            disk.display()
        );
    }
}

/// A template with a blank disk needs the artifact root; without one it
/// fails before writing a definition that would point at a missing image.
#[test]
fn install_without_artifact_root_writes_nothing() {
    let machines = TempDir::new("preset-no-artifacts");
    let error = install(
        Preset::CoCo3DiskBasic,
        &dirs(machines.path(), None, None),
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

/// A slug whose artifact dir outlived its machine is not reused: the new
/// machine's blank disk must not be the old machine's disk.
#[test]
fn new_machine_skips_a_slug_with_leftover_artifacts() {
    let machines = TempDir::new("preset-leftover-machines");
    let artifacts = TempDir::new("preset-leftover-artifacts");
    let old_disk = artifacts.path().join("coco-3-disk-basic").join("disk0.dsk");
    fs::create_dir_all(old_disk.parent().unwrap()).unwrap();
    fs::write(&old_disk, b"old machine's files").unwrap();
    let mut app = ManagerApp::new(
        None,
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
        None,
    );

    app.create_machine_from_preset(Preset::CoCo3DiskBasic);

    assert_eq!(app.entries[0].slug, "coco-3-disk-basic-2");
    let new_disk = artifacts
        .path()
        .join("coco-3-disk-basic-2")
        .join("disk0.dsk");
    assert_eq!(fs::metadata(new_disk).unwrap().len(), 0);
}

/// Plain New skips a leftover artifact dir the same way.
#[test]
fn new_skips_a_slug_with_leftover_artifacts() {
    let machines = TempDir::new("new-leftover-machines");
    let artifacts = TempDir::new("new-leftover-artifacts");
    fs::create_dir_all(artifacts.path().join("coco-3")).unwrap();
    let mut app = ManagerApp::new(
        None,
        Some(machines.path().to_path_buf()),
        Some(artifacts.path().to_path_buf()),
        Vec::new(),
        None,
    );

    app.create_machine_now();

    assert_eq!(app.entries[0].slug, "coco-3-2");
}

/// Seeding over a data directory that outlived its config gives the
/// starter machines fresh slugs instead of the old artifact dirs.
#[test]
fn first_run_skips_slugs_with_leftover_artifacts() {
    let config = TempDir::new("preset-seed-leftover-config");
    let artifacts = TempDir::new("preset-seed-leftover-artifacts");
    fs::create_dir_all(artifacts.path().join("coco-3-disk-basic")).unwrap();
    let machines = config.path().join("machines");

    seed_first_run(&machines, Some(artifacts.path()), None).expect("seeding succeeds");

    let loaded = machine_def::load_all(&machines).expect("seeded machines load");
    let slugs: Vec<&str> = loaded.iter().map(|(slug, _)| slug.as_str()).collect();
    assert_eq!(slugs, ["coco-2-disk-basic", "coco-3-disk-basic-2"]);
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

/// A failure partway through seeding leaves neither the machines directory
/// nor the staging directory behind, so the next launch seeds every starter
/// machine. A dangling symlink at the CoCo 2 machine's artifact dir counts as
/// free to `slug_on_disk` but cannot be created, so the CoCo 3 machine is
/// staged first and the CoCo 2 one then fails.
#[cfg(unix)]
#[test]
fn failed_seeding_is_retried_on_the_next_launch() {
    let config = TempDir::new("preset-seed-retry-config");
    let artifacts = TempDir::new("preset-seed-retry-artifacts");
    let machines = config.path().join("machines");
    let blocked = artifacts.path().join("coco-2-disk-basic");
    std::os::unix::fs::symlink(artifacts.path().join("missing"), &blocked).unwrap();

    seed_first_run(&machines, Some(artifacts.path()), created()).unwrap_err();

    assert!(!machines.exists());
    assert!(!staging_dir(&machines).unwrap().exists());
    assert!(!artifacts.path().join("coco-3-disk-basic").exists());
    fs::remove_file(&blocked).unwrap();
    seed_first_run(&machines, Some(artifacts.path()), created()).expect("retry succeeds");
    let loaded = machine_def::load_all(&machines).expect("seeded machines load");
    let slugs: Vec<&str> = loaded.iter().map(|(slug, _)| slug.as_str()).collect();
    assert_eq!(slugs, ["coco-2-disk-basic", "coco-3-disk-basic"]);
}

/// A staging directory left by a crash is discarded with the empty blank
/// disks it placed, so the starter machines get their plain slugs back. A
/// staged machine's disk that holds data is kept, and its slug skipped.
#[test]
fn seeding_discards_a_crashed_run() {
    let config = TempDir::new("preset-seed-crash-config");
    let artifacts = TempDir::new("preset-seed-crash-artifacts");
    let machines = config.path().join("machines");
    let staging = staging_dir(&machines).unwrap();
    for preset in Preset::FIRST_RUN {
        let slug = machine_def::slugify(preset.name());
        install(
            preset,
            &dirs(&staging, Some(artifacts.path()), None),
            &slug,
            None,
        )
        .unwrap();
    }
    let kept = artifacts.path().join("coco-2-disk-basic").join("disk0.dsk");
    fs::write(&kept, b"user data").unwrap();

    seed_first_run(&machines, Some(artifacts.path()), created()).expect("seeding succeeds");

    assert!(!staging.exists());
    let loaded = machine_def::load_all(&machines).expect("seeded machines load");
    let slugs: Vec<&str> = loaded.iter().map(|(slug, _)| slug.as_str()).collect();
    assert_eq!(slugs, ["coco-2-disk-basic-2", "coco-3-disk-basic"]);
    assert_eq!(fs::read(&kept).unwrap(), b"user data");
}

/// App instances launched together at first run; enough that, unserialized,
/// one would discard another's live staging directory.
const CONCURRENT_LAUNCHES: usize = 4;

/// A launch that finds another instance seeding waits for it, then leaves
/// the machines directory that instance put in place alone. The test holds
/// the lock as the other instance and creates an empty machines directory
/// before releasing it, so any machine in it came from the waiting launch.
/// The launch starts past [`seed_first_run`]'s first check, so it can only
/// see the directory once it holds the lock.
#[test]
fn seeding_waits_for_another_instance() {
    let config = TempDir::new("preset-seed-wait-config");
    let artifacts = TempDir::new("preset-seed-wait-artifacts");
    let machines = config.path().join("machines");
    let lock = lock_seeding(&machines).expect("lock taken");

    let waiting = thread::scope(|scope| {
        let launch = scope.spawn(|| seed_under_lock(&machines, Some(artifacts.path()), created()));
        fs::create_dir(&machines).unwrap();
        drop(lock);
        launch.join().unwrap()
    });

    waiting.expect("the waiting launch succeeds");
    assert_eq!(fs::read_dir(&machines).unwrap().count(), 0);
    assert!(!staging_dir(&machines).unwrap().exists());
}

/// Launches racing on a first run seed the starter machines once, under
/// their plain slugs, and leave no staging directory behind.
#[test]
fn concurrent_first_runs_seed_once() {
    let config = TempDir::new("preset-seed-race-config");
    let artifacts = TempDir::new("preset-seed-race-artifacts");
    let machines = config.path().join("machines");
    let start = Barrier::new(CONCURRENT_LAUNCHES);

    let results: Vec<_> = thread::scope(|scope| {
        let launches: Vec<_> = (0..CONCURRENT_LAUNCHES)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    seed_first_run(&machines, Some(artifacts.path()), created())
                })
            })
            .collect();
        launches
            .into_iter()
            .map(|launch| launch.join().unwrap())
            .collect()
    });

    assert!(results.iter().all(Result::is_ok), "{results:?}");
    let loaded = machine_def::load_all(&machines).expect("seeded machines load");
    let slugs: Vec<&str> = loaded.iter().map(|(slug, _)| slug.as_str()).collect();
    assert_eq!(slugs, ["coco-2-disk-basic", "coco-3-disk-basic"]);
    assert!(!staging_dir(&machines).unwrap().exists());
    assert_eq!(fs::read_dir(artifacts.path()).unwrap().count(), slugs.len());
}

/// Each template boots through the production launch path with the
/// installed ROMs: its DOS ROM loads and its blank floppy mounts. The
/// relative `disk0` is pointed at the placed file, since launch resolves it
/// against the real data directory.
#[test]
fn every_preset_launches() {
    // The test bundle's EOU pair is the release's two images.
    let eou_dir = test_assets::disk(test_assets::disk::EOU_BOOT)
        .parent()
        .expect("test disks live in a directory")
        .to_path_buf();
    for preset in Preset::ALL {
        if !preset.available(Some(&eou_dir)) {
            eprintln!("skipping {preset:?}: the EOU test images are not present");
            continue;
        }
        let machines = TempDir::new("preset-launch-machines");
        let artifacts = TempDir::new("preset-launch-artifacts");
        let slug = machine_def::slugify(preset.name());
        let mut def = install(
            preset,
            &dirs(machines.path(), Some(artifacts.path()), Some(&eou_dir)),
            &slug,
            None,
        )
        .unwrap_or_else(|e| panic!("{preset:?} must install: {e}"));
        for media in [&mut def.media.disk0, &mut def.media.vhd0]
            .into_iter()
            .flatten()
        {
            *media = artifacts
                .path()
                .join(&slug)
                .join(&*media)
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
            "{preset:?}: drive 0 mount"
        );
        assert_eq!(
            app.vhd_paths[0].is_some(),
            def.media.vhd0.is_some(),
            "{preset:?}: VHD mount"
        );
    }
}

/// The EOU template copies both images into the new machine's artifact
/// dir, where its relative media names resolve, and leaves the downloaded
/// ones untouched.
#[test]
fn eou_install_copies_the_images() {
    let machines = TempDir::new("preset-eou-machines");
    let artifacts = TempDir::new("preset-eou-artifacts");
    let eou = eou_fixture();
    let slug = machine_def::slugify(Preset::NitrOS9Eou.name());

    let def = install(
        Preset::NitrOS9Eou,
        &dirs(machines.path(), Some(artifacts.path()), Some(eou.path())),
        &slug,
        None,
    )
    .unwrap();

    assert_eq!(def.media.disk0.as_deref(), Some(EOU_BOOT_DISK));
    assert_eq!(def.media.vhd0.as_deref(), Some(EOU_SYSTEM_VHD));
    assert_eq!(def.peripherals.cartridge.dos_rom(), Some(DosRom::DiskBasic));
    for name in EOU_IMAGES {
        let copy = artifacts.path().join(&slug).join(name);
        assert_eq!(fs::read(&copy).unwrap(), FAKE_EOU_IMAGE, "{name}");
        assert!(eou.path().join(name).is_file(), "{name} stays downloaded");
    }
}

/// Without the downloaded images the EOU template is unavailable and
/// installs nothing: no definition, no artifact dir.
#[test]
fn eou_install_without_images_writes_nothing() {
    let machines = TempDir::new("preset-eou-missing-machines");
    let artifacts = TempDir::new("preset-eou-missing-artifacts");
    let empty = TempDir::new("preset-eou-missing-images");
    let slug = machine_def::slugify(Preset::NitrOS9Eou.name());

    for eou_dir in [None, Some(empty.path())] {
        assert!(!Preset::NitrOS9Eou.available(eou_dir));
        let error = install(
            Preset::NitrOS9Eou,
            &dirs(machines.path(), Some(artifacts.path()), eou_dir),
            &slug,
            None,
        )
        .unwrap_err();
        assert_eq!(error, EOU_NOT_DOWNLOADED);
    }
    assert!(!machine_def::def_path(machines.path(), &slug).exists());
    assert!(!artifacts.path().join(&slug).exists());
    assert!(Preset::CoCo3DiskBasic.available(None));
}
