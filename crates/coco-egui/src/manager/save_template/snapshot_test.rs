use std::fs;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::machine_def::{DriveWireShareDTO, MediaDTO, PrintOutputDTO, ShareAccessDTO};
use crate::manager::presets::Preset;

/// The size of the disk image the file list tests write.
const DISK_BYTES: usize = 184_320;

/// A relative share folder, and the print folder inside it.
const SHARE: &str = "work";
const PRINT_SUBFOLDER: &str = "prints";

/// A share named `path`, read-write.
fn share(path: &str) -> DriveWireShareDTO {
    DriveWireShareDTO {
        name: "work".to_string(),
        path: path.to_string(),
        access: ShareAccessDTO::ReadWrite,
    }
}

/// `def` printing to the folder `folder`.
fn printing_to(mut def: MachineDef, folder: &str) -> MachineDef {
    def.drivewire.printer.output = PrintOutputDTO::Folder;
    def.drivewire.printer.folder = folder.to_string();
    def
}

/// The snapshot takes the new name and date, zeroes the stats, and turns
/// absolute paths into the machine's own folder relative, the ROM and
/// paths elsewhere excepted.
#[test]
fn snapshot_detaches_from_the_machine() {
    let root = TempDir::new("snapshot-detach");
    let dir = root.path().join("machines").join("games");
    let shared = root.path().join("shared").join("tape.cas");
    let shared = shared.to_string_lossy().into_owned();
    let mut def = Preset::CoCo3DiskBasic.def(Some("2020-01-01".to_string()));
    def.stats.starts = 5;
    def.media.disk1 = Some(
        dir.join("sub")
            .join("disk1.dsk")
            .to_string_lossy()
            .into_owned(),
    );
    def.media.tape = Some(shared.clone());
    def.hardware.rom = Some(dir.join("coco3.rom").to_string_lossy().into_owned());
    let def = printing_to(def, &dir.join("prints").to_string_lossy());

    let snapshot = snapshot(
        &def,
        Some(&dir),
        "Mine".to_string(),
        Some("2026-10-23".to_string()),
    );

    assert_eq!(snapshot.name, "Mine");
    assert_eq!(snapshot.created.as_deref(), Some("2026-10-23"));
    assert_eq!(snapshot.stats, StatsDTO::default());
    assert_eq!(snapshot.media.disk0.as_deref(), Some("disk0.dsk"));
    let disk1 = snapshot.media.disk1.as_deref().unwrap();
    assert_eq!(Path::new(disk1), Path::new("sub").join("disk1.dsk"));
    assert_eq!(snapshot.media.tape.as_deref(), Some(shared.as_str()));
    assert_eq!(snapshot.hardware.rom, def.hardware.rom);
    assert_eq!(snapshot.drivewire.printer.folder, "prints");
}

/// A print folder that is the machine's data folder itself becomes the
/// relative `.`, not a path back at the machine.
#[test]
fn snapshot_makes_a_print_folder_of_the_data_folder_relative() {
    let root = TempDir::new("snapshot-print-dot");
    let dir = root.path().join("machines").join("games");
    let def = printing_to(Preset::CoCo3DiskBasic.def(None), &dir.to_string_lossy());

    let snapshot = snapshot(&def, Some(&dir), "Mine".to_string(), None);

    assert_eq!(snapshot.drivewire.printer.folder, DATA_FOLDER);
}

/// The file list says what is copied (with its size), shared, or missing.
#[test]
fn file_lines_say_what_happens_to_each_file() {
    let dir = TempDir::new("save-template-lines");
    let shared = TempDir::new("save-template-lines-shared");
    let tape = shared
        .path()
        .join("tape.cas")
        .to_string_lossy()
        .into_owned();
    fs::write(dir.path().join("disk0.dsk"), vec![0; DISK_BYTES]).unwrap();
    let mut def = Preset::CoCo3DiskBasic.def(None);
    def.media.disk1 = Some("disk1.dsk".to_string());
    def.media.tape = Some(tape.clone());

    let lines: Vec<String> = file_lines(&def, Some(dir.path()))
        .iter()
        .map(FileLine::text)
        .collect();

    assert_eq!(
        lines,
        [
            "disk0.dsk (185 KB) is copied".to_string(),
            "disk1.dsk is missing from the machine's folder".to_string(),
            format!("{tape} stays shared"),
        ]
    );
}

/// A relative path climbing out of the machine's folder is never looked
/// at, and keeps Save off.
#[test]
fn file_lines_refuse_a_path_outside_the_folder() {
    let root = TempDir::new("save-template-outside");
    let dir = root.path().join("machine");
    fs::create_dir_all(&dir).unwrap();
    // Present, so only the path itself can make it blocked.
    fs::write(root.path().join("x.dsk"), b"disk").unwrap();
    let mut def = Preset::CoCo3DiskBasic.def(None);
    def.media.disk0 = None;
    def.media.disk1 = Some("../x.dsk".to_string());

    let lines = file_lines(&def, Some(&dir));

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].fate, FileFate::Outside);
    assert!(lines[0].fate.blocks_save());
    assert_eq!(lines[0].text(), "../x.dsk is outside the machine's folder");
}

/// Only a missing or outside file keeps Save off.
#[test]
fn only_missing_and_outside_files_block_saving() {
    assert!(FileFate::Missing.blocks_save());
    assert!(FileFate::Outside.blocks_save());
    assert!(!FileFate::Copied(0).blocks_save());
    assert!(!FileFate::Shared.blocks_save());
}

/// A print folder inside a share is left out of that share's size.
#[test]
fn file_lines_leave_the_print_folder_out_of_a_share() {
    const NOTES: &[u8] = b"notes";
    const PRINT_JOB: &[u8] = b"a long print job";
    let dir = TempDir::new("save-template-share-prints");
    let prints = dir.path().join(SHARE).join(PRINT_SUBFOLDER);
    fs::create_dir_all(&prints).unwrap();
    fs::write(dir.path().join(SHARE).join("notes.txt"), NOTES).unwrap();
    fs::write(prints.join("job.txt"), PRINT_JOB).unwrap();
    let mut def = Preset::CoCo3DriveWire.def(None);
    def.media = MediaDTO::default();
    def.drivewire.shares = vec![share(SHARE)];
    let def = printing_to(def, &format!("{SHARE}/{PRINT_SUBFOLDER}"));

    let lines = file_lines(&def, Some(dir.path()));

    let share_line = lines.iter().find(|line| line.path == SHARE).unwrap();
    assert_eq!(share_line.fate, FileFate::Copied(NOTES.len() as u64));
}

/// Sizes read in kilobytes below a megabyte, in megabytes from there.
#[test]
fn sizes_switch_to_megabytes() {
    assert_eq!(size_text(0), "0 KB");
    assert_eq!(size_text(999_999), "1000 KB");
    assert_eq!(size_text(128_000_000), "128.0 MB");
}
