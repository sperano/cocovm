//! Validate whatever known-named ROMs are present in the XDG ROM directory
//! against the MAME-derived manifest. Absent files are skipped; a present
//! file with wrong contents is a hard failure — it means a corrupt dump or
//! a manifest typo, and every boot test downstream would chase ghosts.

use coco_core::rom_db::{self, Validation};

#[test]
fn local_roms_match_manifest() {
    let roms_dir = test_assets::roms_dir();
    let mut checked = 0;
    for known in rom_db::SYSTEM_ROMS {
        let path = roms_dir.join(known.file);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match rom_db::validate(known.file, &bytes) {
            Validation::Verified(found) => {
                assert_eq!(found.file, known.file, "content matches a different entry");
                checked += 1;
            }
            other => panic!("{}: {other:?}", path.display()),
        }
    }
    println!(
        "verified {checked} of {} known ROMs present locally",
        rom_db::SYSTEM_ROMS.len()
    );
}
