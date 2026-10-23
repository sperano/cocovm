use std::fs;

use super::*;
use crate::machine_def::PrintOutputDTO;
use crate::machine_def::tests::TempDir;
use crate::manager::presets::Preset;

/// The snapshot takes the new name and date, zeroes the stats, and turns
/// absolute paths into the machine's own folder relative, the ROM and
/// paths elsewhere excepted.
#[test]
fn snapshot_detaches_from_the_machine() {
    let dir = Path::new("/data/machines/games");
    let mut def = Preset::CoCo3DiskBasic.def(Some("2020-01-01".to_string()));
    def.stats.starts = 5;
    def.media.disk1 = Some("/data/machines/games/sub/disk1.dsk".to_string());
    def.media.tape = Some("/shared/tape.cas".to_string());
    def.hardware.rom = Some("/data/machines/games/coco3.rom".to_string());
    def.drivewire.printer.output = PrintOutputDTO::Folder;
    def.drivewire.printer.folder = "/data/machines/games/prints".to_string();

    let snapshot = snapshot(
        &def,
        Some(dir),
        "Mine".to_string(),
        Some("2026-10-23".to_string()),
    );

    assert_eq!(snapshot.name, "Mine");
    assert_eq!(snapshot.created.as_deref(), Some("2026-10-23"));
    assert_eq!(snapshot.stats, StatsDTO::default());
    assert_eq!(snapshot.media.disk0.as_deref(), Some("disk0.dsk"));
    assert_eq!(snapshot.media.disk1.as_deref(), Some("sub/disk1.dsk"));
    assert_eq!(snapshot.media.tape.as_deref(), Some("/shared/tape.cas"));
    assert_eq!(snapshot.hardware.rom, def.hardware.rom);
    assert_eq!(snapshot.drivewire.printer.folder, "prints");
}

/// The file list says what is copied (with its size), shared, or missing.
#[test]
fn file_lines_say_what_happens_to_each_file() {
    let dir = TempDir::new("save-template-lines");
    fs::write(dir.path().join("disk0.dsk"), vec![0; 184_320]).unwrap();
    let mut def = Preset::CoCo3DiskBasic.def(None);
    def.media.disk1 = Some("disk1.dsk".to_string());
    def.media.tape = Some("/shared/tape.cas".to_string());

    let lines: Vec<String> = file_lines(&def, Some(dir.path()))
        .iter()
        .map(FileLine::text)
        .collect();

    assert_eq!(
        lines,
        [
            "disk0.dsk (185 KB) is copied",
            "disk1.dsk is missing from the machine's folder",
            "/shared/tape.cas stays shared",
        ]
    );
}

/// Sizes read in kilobytes below a megabyte, in megabytes from there.
#[test]
fn sizes_switch_to_megabytes() {
    assert_eq!(size_text(0), "0 KB");
    assert_eq!(size_text(999_999), "1000 KB");
    assert_eq!(size_text(128_000_000), "128.0 MB");
}
