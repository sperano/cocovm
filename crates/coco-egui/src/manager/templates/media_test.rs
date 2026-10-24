use super::*;
use crate::machine_def::tests::TempDir;
use crate::machine_def::{CartridgeDTO, DriveWireShareDTO, MediaDTO, ShareAccessDTO};
use crate::manager::presets::Preset;

/// A definition naming a relative disk, a relative VHD in a subfolder, an
/// absolute tape, a relative share, and a relative print folder.
fn mixed_def() -> MachineDef {
    let mut def = Preset::CoCo3DiskBasic.def(None);
    def.media.vhd0 = Some("disks/hd0.vhd".to_string());
    def.media.tape = Some("/shared/games.cas".to_string());
    def.drivewire.shares = vec![DriveWireShareDTO {
        name: "work".to_string(),
        path: "work".to_string(),
        access: ShareAccessDTO::ReadWrite,
    }];
    def.drivewire.printer.output = PrintOutputDTO::Folder;
    def.drivewire.printer.folder = "prints".to_string();
    def
}

/// Relative files and share folders travel; absolute paths and the print
/// folder don't.
#[test]
fn relative_media_lists_what_travels() {
    let media = relative_media(&mixed_def());
    assert_eq!(media.files, ["disk0.dsk", "disks/hd0.vhd"]);
    assert_eq!(media.folders, ["work"]);
}

/// A cartridge ROM named relative to the data folder is a file to copy.
#[test]
fn relative_cartridge_rom_travels() {
    let mut def = Preset::CoCo3DriveWire.def(None);
    def.peripherals.cartridge = CartridgeDTO::ROMPak {
        path: "game.ccc".to_string(),
    };
    assert_eq!(relative_media(&def).files, ["game.ccc"]);
}

/// The copy writes every file and folder, keeps subfolders, and measures
/// what it copies.
#[test]
fn copy_writes_files_and_folders() {
    let from = TempDir::new("media-copy-from");
    let to = TempDir::new("media-copy-to");
    fs::write(from.path().join("disk0.dsk"), b"disk").unwrap();
    fs::create_dir_all(from.path().join("disks")).unwrap();
    fs::write(from.path().join("disks/hd0.vhd"), b"hard disk").unwrap();
    fs::create_dir_all(from.path().join("work/sub")).unwrap();
    fs::write(from.path().join("work/sub/notes.txt"), b"notes").unwrap();
    let target = to.path().join("new");
    let copy = MediaCopy::new(&mixed_def(), from.path().to_path_buf(), target.clone()).unwrap();

    assert_eq!(copy.bytes(), 4 + 9 + 5);
    assert!(!copy.in_background());
    copy.run().unwrap();

    assert_eq!(fs::read(target.join("disk0.dsk")).unwrap(), b"disk");
    assert_eq!(
        fs::read(target.join("disks/hd0.vhd")).unwrap(),
        b"hard disk"
    );
    assert_eq!(
        fs::read(target.join("work/sub/notes.txt")).unwrap(),
        b"notes"
    );
    assert!(!target.join("prints").exists());
}

/// A relative share folder, and a print folder inside it.
const SHARE: &str = "work";
const PRINT_SUBFOLDER: &str = "prints";
const NOTES: &[u8] = b"notes";
const PRINT_JOB: &[u8] = b"a long print job";

/// A DriveWire machine sharing the relative folder `SHARE` and printing to
/// `print_folder`, with no disk media.
fn sharing_and_printing(print_folder: &str) -> MachineDef {
    let mut def = Preset::CoCo3DriveWire.def(None);
    def.media = MediaDTO::default();
    def.drivewire.shares = vec![DriveWireShareDTO {
        name: "work".to_string(),
        path: SHARE.to_string(),
        access: ShareAccessDTO::ReadWrite,
    }];
    def.drivewire.printer.output = PrintOutputDTO::Folder;
    def.drivewire.printer.folder = print_folder.to_string();
    def
}

/// A data folder holding the share with a note and, inside it, a print job.
fn data_folder_with_prints() -> TempDir {
    let from = TempDir::new("media-share-prints");
    let prints = from.path().join(SHARE).join(PRINT_SUBFOLDER);
    fs::create_dir_all(&prints).unwrap();
    fs::write(from.path().join(SHARE).join("notes.txt"), NOTES).unwrap();
    fs::write(prints.join("job.txt"), PRINT_JOB).unwrap();
    from
}

/// A print folder that is also a share is the print folder: it isn't
/// listed as a share folder to copy, however the path is spelled.
#[test]
fn a_print_folder_equal_to_a_share_is_not_a_share_folder() {
    for spelling in [SHARE, "./work", "work/."] {
        let media = relative_media(&sharing_and_printing(spelling));
        assert!(media.folders.is_empty(), "{spelling}");
        assert!(media.files.is_empty(), "{spelling}");
        assert_eq!(media.print_folder.as_deref(), Some(spelling));
    }
}

/// A print folder inside a share is neither copied nor counted with it.
#[test]
fn a_print_folder_inside_a_share_is_not_copied() {
    let from = data_folder_with_prints();
    let to = TempDir::new("media-share-prints-to");
    let target = to.path().join("new");
    let def = sharing_and_printing(&format!("{SHARE}/{PRINT_SUBFOLDER}"));
    let copy = MediaCopy::new(&def, from.path().to_path_buf(), target.clone()).unwrap();

    assert_eq!(copy.bytes(), NOTES.len() as u64);
    copy.run().unwrap();

    assert_eq!(
        fs::read(target.join(SHARE).join("notes.txt")).unwrap(),
        NOTES
    );
    assert!(!target.join(SHARE).join(PRINT_SUBFOLDER).exists());
}

/// A share that is a symlink to a folder elsewhere is left alone: it is
/// neither copied nor counted, so a link can't pull in files from outside
/// the data folder.
#[cfg(unix)]
#[test]
fn a_share_that_links_outside_is_not_copied() {
    let from = TempDir::new("media-share-link-from");
    let outside = TempDir::new("media-share-link-outside");
    let to = TempDir::new("media-share-link-to");
    fs::write(outside.path().join("secret.txt"), PRINT_JOB).unwrap();
    std::os::unix::fs::symlink(outside.path(), from.path().join(SHARE)).unwrap();
    let target = to.path().join("new");
    let mut def = sharing_and_printing("prints");
    def.drivewire.printer.output = PrintOutputDTO::Off;
    let copy = MediaCopy::new(&def, from.path().to_path_buf(), target.clone()).unwrap();

    assert_eq!(copy.bytes(), 0);
    copy.run().unwrap();

    assert!(!target.join(SHARE).exists());
}

/// A missing file fails the copy; a missing share folder is skipped.
#[test]
fn copy_fails_on_a_missing_file_only() {
    let from = TempDir::new("media-missing-from");
    let to = TempDir::new("media-missing-to");
    fs::write(from.path().join("disk0.dsk"), b"disk").unwrap();
    let copy =
        MediaCopy::new(&mixed_def(), from.path().to_path_buf(), to.path().join("t")).unwrap();

    let error = copy.run().unwrap_err();

    assert!(error.contains("disks/hd0.vhd"), "{error}");
    fs::create_dir_all(from.path().join("disks")).unwrap();
    fs::write(from.path().join("disks/hd0.vhd"), b"").unwrap();
    copy.run().expect("the missing share folder is skipped");
}

/// A path climbing out of the data folder is refused before anything is
/// copied.
#[test]
fn copy_refuses_a_path_leaving_the_folder() {
    let mut def = Preset::CoCo3DriveWire.def(None);
    def.media.disk1 = Some("../other/disk0.dsk".to_string());
    let error = MediaCopy::new(&def, PathBuf::from("/a"), PathBuf::from("/b")).unwrap_err();
    assert!(error.contains("../other/disk0.dsk"), "{error}");
}

/// A copy larger than the threshold runs in the background.
#[test]
fn large_copy_runs_in_the_background() {
    let from = TempDir::new("media-large-from");
    fs::File::create(from.path().join("disk0.dsk"))
        .and_then(|file| file.set_len(BACKGROUND_COPY_BYTES + 1))
        .unwrap();
    let def = Preset::CoCo3DiskBasic.def(None);
    let copy = MediaCopy::new(&def, from.path().to_path_buf(), from.path().join("t")).unwrap();
    assert!(copy.in_background());
}

/// The files a definition names that its media folder lacks.
#[test]
fn missing_files_lists_absent_files() {
    let dir = TempDir::new("media-missing-files");
    let def = mixed_def();
    assert_eq!(
        missing_files(&def, dir.path()),
        ["disk0.dsk", "disks/hd0.vhd"]
    );
    fs::write(dir.path().join("disk0.dsk"), b"").unwrap();
    assert_eq!(missing_files(&def, dir.path()), ["disks/hd0.vhd"]);
}

/// Claim skips a slug whose folder exists and creates the next one.
#[test]
fn claim_skips_an_existing_folder() {
    let root = TempDir::new("media-claim");
    fs::create_dir(root.path().join("coco")).unwrap();

    let slug = claim(root.path(), "coco", &|_| false).unwrap();

    assert_eq!(slug, "coco-2");
    assert!(root.path().join("coco-2").is_dir());
}
