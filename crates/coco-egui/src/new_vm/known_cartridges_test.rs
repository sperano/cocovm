use super::*;

use crate::machine_def::tests::TempDir;

const ANDRONE_FILE: &str = "Androne (1983) (26-3096) (Tandy).ccc";
const TETRIS_FILE: &str = "Tetris (1987) (26-3163) (Tandy) (Coco 1-2) (Coco 3).ccc";

#[test]
fn row_details_lists_year_vendor_catalog_and_machine() {
    let thexder = KNOWN_CARTRIDGE_ROMS
        .iter()
        .find(|known| known.name == "Thexder")
        .unwrap();
    assert_eq!(row_details(thexder), "1987 · Tandy · 26-3072 · CoCo 3");
    let androne = KNOWN_CARTRIDGE_ROMS
        .iter()
        .find(|known| known.name == "Androne")
        .unwrap();
    assert_eq!(row_details(androne), "1983 · Tandy · 26-3096");
}

#[test]
fn missing_dir_is_empty() {
    let dir = TempDir::new("known-cartridges");
    assert!(bundled_cartridges_in(&dir.path().join("absent")).is_empty());
}

#[test]
fn dir_without_known_files_is_empty() {
    let dir = TempDir::new("known-cartridges");
    std::fs::write(dir.path().join("not-a-known-rom.ccc"), b"\x00").unwrap();
    assert!(bundled_cartridges_in(dir.path()).is_empty());
}

#[test]
fn finds_exactly_the_bundled_files_present_sorted_by_name() {
    let dir = TempDir::new("known-cartridges");
    // Content is never read — dummy bytes are fine.
    std::fs::write(dir.path().join(TETRIS_FILE), b"\x00").unwrap();
    std::fs::write(dir.path().join(ANDRONE_FILE), b"\x00").unwrap();

    let found = bundled_cartridges_in(dir.path());
    let names: Vec<&str> = found.iter().map(|(known, _)| known.name).collect();
    assert_eq!(names, vec!["Androne", "Tetris"], "sorted by name");
    assert_eq!(found[0].1, dir.path().join(ANDRONE_FILE));
    assert_eq!(found[1].1, dir.path().join(TETRIS_FILE));
}

/// Walks the installed asset bundle; skips when it isn't present. Guards the
/// hand-maintained `bundled_file` names: a typo hides the cartridge from the
/// submenu, and a name on the wrong entry loads a different cartridge.
#[test]
fn every_bundled_file_is_present_and_identifies_as_its_entry() {
    let Some(dir) = crate::paths::cartridges_dir().filter(|dir| dir.is_dir()) else {
        eprintln!(
            "skipping every_bundled_file_is_present_and_identifies_as_its_entry: \
             assets/cartridges not present"
        );
        return;
    };
    for known in KNOWN_CARTRIDGE_ROMS {
        let Some(file) = known.bundled_file else {
            continue;
        };
        let bytes = std::fs::read(dir.join(file)).unwrap_or_else(|err| panic!("{file}: {err}"));
        let identified = coco_core::rom_db::identify_cartridge(&bytes)
            .unwrap_or_else(|| panic!("{file}: contents not in the manifest"));
        // KNOWN_CARTRIDGE_ROMS is a const, so compare by value, not pointer.
        assert_eq!(identified, known, "{file}: identifies as a different entry");
    }
}
