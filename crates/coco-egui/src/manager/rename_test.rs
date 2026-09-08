use super::*;

use coco_core::MachineConfig;

use crate::machine_def::tests::TempDir;

fn manager_fixture(name: &str, slug: &str) -> (TempDir, TempDir, ManagerApp) {
    let machines_dir = TempDir::new("rename-transaction-machines");
    let artifacts_root = TempDir::new("rename-transaction-artifacts");
    let def =
        machine_def::MachineDef::from_config(name.to_string(), None, &MachineConfig::default());
    machine_def::save(machines_dir.path(), slug, &def).expect("seed definition");
    let manager = ManagerApp::new(
        None,
        Some(machines_dir.path().to_path_buf()),
        Some(artifacts_root.path().to_path_buf()),
        vec![super::super::MachineEntry::new(slug.to_string(), def)],
        None,
    );
    (machines_dir, artifacts_root, manager)
}

#[test]
fn running_machine_rename_keeps_the_vm_and_window_session() {
    let (machines_dir, artifacts_root, mut manager) = manager_fixture("Alpha", "alpha");
    manager.start_vm(0);
    assert!(
        manager.entries[0].vm.is_some(),
        "VM must launch: {:?}",
        manager.entries[0].launch_error
    );
    let old_artifacts = artifacts_root.path().join("alpha");
    fs::create_dir_all(&old_artifacts).expect("create old artifacts");
    let capture_path = old_artifacts.join("printout.txt");
    let vhd_path = old_artifacts.join("hd0.vhd");
    let drivewire_path = old_artifacts.join("dw0.dsk");
    fs::write(&vhd_path, vec![0_u8; coco_core::vhd::SECTOR_SIZE]).expect("seed VHD");
    fs::write(
        &drivewire_path,
        vec![0_u8; coco_core::drivewire::SECTOR_SIZE],
    )
    .expect("seed DriveWire disk");
    let vm = manager.entries[0].vm.as_mut().unwrap();
    vm.start_print_capture(capture_path);
    vm.insert_vhd(0, vhd_path);
    vm.enable_drivewire(false);
    vm.insert_dw_disk(0, drivewire_path);
    assert!(vm.cart_error.is_none(), "managed media must mount");
    let window_session = manager.entries[0].window_session;

    manager.queue_rename("alpha".to_string(), "Alpha Sokoban".to_string());
    manager.apply_pending_rename();

    let entry = &mut manager.entries[0];
    assert_eq!(entry.slug, "alpha-sokoban");
    assert_eq!(entry.def.name, "Alpha Sokoban");
    assert!(entry.vm.is_some(), "rename must not stop the VM");
    assert_eq!(entry.window_session, window_session);
    assert_eq!(
        entry.vm.as_ref().unwrap().print_capture_path.as_deref(),
        Some(
            artifacts_root
                .path()
                .join("alpha-sokoban/printout.txt")
                .as_path()
        )
    );
    let vm = entry.vm.as_ref().unwrap();
    assert_eq!(
        vm.vhd_paths[0].as_deref(),
        Some(
            artifacts_root
                .path()
                .join("alpha-sokoban/hd0.vhd")
                .as_path()
        )
    );
    assert_eq!(
        vm.dw_paths[0].as_deref(),
        Some(
            artifacts_root
                .path()
                .join("alpha-sokoban/dw0.dsk")
                .as_path()
        )
    );
    assert!(vm.machine.bus.vhd.is_mounted(0));
    assert!(vm.machine.bus.drivewire.as_ref().unwrap().is_mounted(0));
    assert!(machines_dir.path().join("alpha-sokoban.toml").is_file());
    assert!(!machines_dir.path().join("alpha.toml").exists());
    assert!(artifacts_root.path().join("alpha-sokoban").is_dir());
    assert!(!artifacts_root.path().join("alpha").exists());
}

#[test]
fn suspended_checkpoint_paths_follow_the_new_slug_and_resume() {
    let (machines_dir, artifacts_root, mut manager) = manager_fixture("Alpha", "alpha");
    let old_artifacts = artifacts_root.path().join("alpha");
    fs::create_dir_all(&old_artifacts).expect("create old artifacts");
    let tape_path = old_artifacts.join("tape.cas");
    fs::write(&tape_path, b"tape").expect("seed tape");
    manager.entries[0].def.media.tape = Some(tape_path.to_string_lossy().into_owned());
    machine_def::save(machines_dir.path(), "alpha", &manager.entries[0].def)
        .expect("save tape definition");
    manager.start_vm(0);
    manager.suspend_vm(0);
    assert!(
        manager.entries[0].suspended,
        "VM must suspend: {:?}",
        manager.entries[0].launch_error
    );
    manager.entries[0].vm = None;

    manager.queue_rename("alpha".to_string(), "Alpha Suspended".to_string());
    manager.apply_pending_rename();

    let new_artifacts = artifacts_root.path().join("alpha-suspended");
    let state_path = new_artifacts.join(SUSPEND_STATE_FILE);
    let payload = coco_core::snapshot::load(&fs::read(&state_path).expect("read checkpoint"))
        .expect("decode checkpoint");
    assert_eq!(
        payload.media.tape.unwrap().path,
        new_artifacts.join("tape.cas")
    );
    manager.resume_vm(0);
    assert!(
        manager.entries[0].is_running(),
        "renamed checkpoint must resume"
    );
    assert!(manager.entries[0].launch_error.is_none());
}

#[test]
fn artifact_collision_chooses_an_unused_slug_before_writing() {
    let (_machines_dir, artifacts_root, mut manager) = manager_fixture("Alpha", "alpha");
    fs::create_dir_all(artifacts_root.path().join("target")).expect("reserve target artifacts");

    manager.queue_rename("alpha".to_string(), "Target".to_string());
    manager.apply_pending_rename();

    assert_eq!(manager.entries[0].slug, "target-2");
}

#[test]
fn recovery_finishes_a_prepared_rename() {
    let (machines_dir, artifacts_root, manager) = manager_fixture("Alpha", "alpha");
    fs::create_dir_all(artifacts_root.path().join("alpha")).expect("create artifacts");
    let plan = manager
        .rename_plan(0, "Recovered Name".to_string())
        .expect("build plan");
    prepare(&plan).expect("prepare rename");

    recover_pending_rename(machines_dir.path(), Some(artifacts_root.path()))
        .expect("recover rename");

    assert!(machines_dir.path().join("recovered-name.toml").is_file());
    assert!(!machines_dir.path().join("alpha.toml").exists());
    assert!(artifacts_root.path().join("recovered-name").is_dir());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE).exists());
}

#[test]
fn failed_artifact_move_discards_preparation_and_keeps_the_source() {
    let (machines_dir, artifacts_root, manager) = manager_fixture("Alpha", "alpha");
    let old_artifacts = artifacts_root.path().join("alpha");
    fs::create_dir_all(&old_artifacts).expect("create source artifacts");
    let plan = manager
        .rename_plan(0, "Blocked Name".to_string())
        .expect("build plan");
    let blocked_target = plan.new_artifacts.as_ref().unwrap();
    fs::create_dir_all(blocked_target).expect("block destination");
    fs::write(blocked_target.join("occupied"), b"occupied").expect("occupy destination");

    let error = perform(&plan).expect_err("occupied destination must fail");

    assert!(error.contains(&old_artifacts.display().to_string()));
    assert!(machines_dir.path().join("alpha.toml").is_file());
    assert!(old_artifacts.is_dir());
    assert!(!plan.staged_config.exists());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE).exists());
}

#[test]
fn failed_config_install_rolls_the_artifact_move_back() {
    let (machines_dir, artifacts_root, manager) = manager_fixture("Alpha", "alpha");
    let old_artifacts = artifacts_root.path().join("alpha");
    fs::create_dir_all(&old_artifacts).expect("create source artifacts");
    let plan = manager
        .rename_plan(0, "Blocked Config".to_string())
        .expect("build plan");
    fs::create_dir(&plan.backup_config).expect("block config backup path");

    let error = perform(&plan).expect_err("blocked config install must fail");

    assert!(error.contains(&plan.old_config.display().to_string()));
    assert!(machines_dir.path().join("alpha.toml").is_file());
    assert!(old_artifacts.is_dir());
    assert!(!plan.new_artifacts.as_ref().unwrap().exists());
    assert!(!plan.staged_config.exists());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE).exists());
}

#[test]
fn recovery_finishes_after_artifacts_have_moved() {
    let (machines_dir, artifacts_root, manager) = manager_fixture("Alpha", "alpha");
    fs::create_dir_all(artifacts_root.path().join("alpha")).expect("create artifacts");
    let plan = manager
        .rename_plan(0, "Moved Name".to_string())
        .expect("build plan");
    prepare(&plan).expect("prepare rename");
    assert!(move_artifacts(&plan).expect("move artifacts"));

    recover_pending_rename(machines_dir.path(), Some(artifacts_root.path()))
        .expect("recover rename");

    assert!(machines_dir.path().join("moved-name.toml").is_file());
    assert!(artifacts_root.path().join("moved-name").is_dir());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE).exists());
}

#[test]
fn recovery_cleans_up_after_target_config_install() {
    let (machines_dir, artifacts_root, manager) = manager_fixture("Alpha", "alpha");
    fs::create_dir_all(artifacts_root.path().join("alpha")).expect("create artifacts");
    let plan = manager
        .rename_plan(0, "Installed Name".to_string())
        .expect("build plan");
    prepare(&plan).expect("prepare rename");
    assert!(move_artifacts(&plan).expect("move artifacts"));
    install_config(&plan).expect("install target config");

    recover_pending_rename(machines_dir.path(), Some(artifacts_root.path()))
        .expect("recover cleanup");

    assert!(machines_dir.path().join("installed-name.toml").is_file());
    assert!(!plan.backup_config.exists());
    assert!(!machines_dir.path().join(RENAME_JOURNAL_FILE).exists());
}
