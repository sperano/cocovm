use super::*;
use coco_core::MachineConfig;

/// Scratch directory holding only the fixture files a given test writes
/// into it, under `target/` (git-ignored, unlike the workspace `roms/`
/// this test also reads from for the system ROM and the FD-502's
/// `disk11.rom` — both real, local-only dumps per the project's
/// `./roms` convention, same as every other test that mounts an FD-502).
fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-save-state")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A machine with a ROM pak (direct fixture) and a floppy (direct
/// fixture) both mounted through a MultiPak — the FD-502 and a plain
/// cartridge share the single cartridge port on real hardware, so an MPI
/// is the only way to combine them — produces [`MediaRefs`] whose hashes
/// match [`snapshot::sha256_file`] of every file involved: the two
/// fixtures this test wrote, the real `roms/coco3.rom` system ROM, and
/// the real `roms/disk11.rom` the FD-502 always loads
/// ([`disk_basic_rom_path`]).
#[test]
fn build_media_refs_hashes_match_the_mounted_files() {
    let roms_dir = dev_roms_dir();
    let rom_path = roms_dir.join("coco3.rom");
    let rom = std::fs::read(&rom_path)
        .expect("roms/coco3.rom is required (git-ignored, local-only)")
        .into_boxed_slice();
    assert!(
        disk_basic_rom_path().is_file(),
        "roms/disk11.rom is required (git-ignored, local-only)"
    );

    let dir = scratch_dir("media-refs");
    let cart_path = dir.join("game.rom");
    std::fs::write(&cart_path, vec![0x11u8; 0x4000]).expect("write cart fixture");
    let disk_path = dir.join("disk0.dsk");
    std::fs::write(&disk_path, Vec::<u8>::new()).expect("write disk fixture");

    let mut app = CocoApp::new(
        MachineConfig::default(),
        rom,
        RomSource::File(rom_path.clone()),
        None,
        [None, None],
        [None, None],
        std::array::from_fn(|_| None),
        false,
        false,
        false,
    );
    app.insert_multipak();
    app.mpi_insert_rompak(0, cart_path.clone());
    app.mpi_insert_fd502(3);
    assert!(app.cart_error.is_none(), "mounting the MPI slots: {:?}", app.cart_error);
    app.insert_disk(0, disk_path.clone());
    assert!(app.cart_error.is_none(), "mounting the disk: {:?}", app.cart_error);

    let media = app.build_media_refs().expect("build_media_refs should succeed");

    let system_rom = media.system_rom.as_ref().expect("system ROM must be recorded");
    assert_eq!(system_rom.path, rom_path);
    // Hashes the boot-time bytes the running machine actually has
    // (`Self::system_rom_media_ref`'s fix — phase-5 review item 13), not
    // a fresh re-read of the file; they agree here only because the file
    // hasn't changed since boot, which is exactly what this asserts.
    assert_eq!(system_rom.sha256, snapshot::sha256_hex(&app.machine.bus.rom));

    assert_eq!(media.cart_roms.len(), 2, "the ROM pak and the FD-502's own ROM");
    let cart_ref = media
        .cart_roms
        .iter()
        .find(|r| r.mpi_slot == Some(0))
        .expect("the ROM pak's slot must be recorded");
    assert_eq!(cart_ref.rom.path, cart_path);
    assert_eq!(cart_ref.rom.sha256, snapshot::sha256_file(&cart_path).unwrap());
    let fd502_ref = media
        .cart_roms
        .iter()
        .find(|r| r.mpi_slot == Some(3))
        .expect("the FD-502's slot must be recorded");
    assert_eq!(fd502_ref.rom.path, disk_basic_rom_path());
    assert_eq!(
        fd502_ref.rom.sha256,
        snapshot::sha256_file(&disk_basic_rom_path()).unwrap()
    );

    let disk0 = media.disks[0].as_ref().expect("drive 0 must be recorded");
    assert_eq!(disk0.path, disk_path);
    assert_eq!(disk0.sha256, snapshot::sha256_file(&disk_path).unwrap());
    assert!(media.disks[1].is_none());
}
