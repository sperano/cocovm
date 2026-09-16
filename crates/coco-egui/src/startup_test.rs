use std::path::PathBuf;

use super::*;

#[test]
fn unpack_assets_extracts_gzipped_tar_into_dest() {
    let dest =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-test-assets/unpack");
    let _ = std::fs::remove_dir_all(&dest);

    // Build a cocovm-assets-shaped tarball in memory: roms/ and images/.
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tarball = tar::Builder::new(gz);
    for (path, contents) in [
        ("roms/test.rom", &b"\xAA\xBB"[..]),
        ("images/blank.dsk", &b"\x00\x01"[..]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tarball.append_data(&mut header, path, contents).unwrap();
    }
    let bytes = tarball.into_inner().unwrap().finish().unwrap();

    unpack_assets(&bytes[..], &dest).unwrap();
    assert_eq!(
        std::fs::read(dest.join("roms/test.rom")).unwrap(),
        b"\xAA\xBB"
    );
    assert_eq!(
        std::fs::read(dest.join("images/blank.dsk")).unwrap(),
        b"\x00\x01"
    );
}

#[test]
fn missing_bundled_roms_names_only_the_absent_files() {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-test-assets/missing");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        missing_bundled_roms(&dir),
        BUNDLED_ROMS.to_vec(),
        "no dir: all missing"
    );

    std::fs::create_dir_all(&dir).unwrap();
    for name in BUNDLED_ROMS {
        std::fs::write(dir.join(name), b"\xAA").unwrap();
    }
    assert!(missing_bundled_roms(&dir).is_empty());

    std::fs::remove_file(dir.join("sp0256-al2.rom")).unwrap();
    assert_eq!(missing_bundled_roms(&dir), vec!["sp0256-al2.rom"]);
}

#[test]
fn is_rom_file_accepts_roms_and_rejects_appledouble_siblings() {
    assert!(is_rom_file("coco3.rom"));
    assert!(is_rom_file("extbas11.rom"));
    // macOS resource forks unpacked from the asset tarball.
    assert!(!is_rom_file("._coco3.rom"));
    assert!(!is_rom_file(".DS_Store"));
    assert!(!is_rom_file("blank.dsk"));
    assert!(!is_rom_file("rom"));
}

#[test]
fn is_cartridge_file_accepts_ccc_and_rejects_appledouble_siblings() {
    assert!(is_cartridge_file("Atom (1983) (26-3149) (Tandy).ccc"));
    assert!(!is_cartridge_file("._Atom (1983) (26-3149) (Tandy).ccc"));
    assert!(!is_cartridge_file(".DS_Store"));
    assert!(!is_cartridge_file("coco3.rom"));
    assert!(!is_cartridge_file("ccc"));
}

#[test]
fn inventory_pluralizes_every_count() {
    let info = |roms, cartridges, machines| StartupInfo {
        roms,
        cartridges,
        machines,
        renderer: String::new(),
    };
    assert_eq!(
        info(8, 126, 7).inventory(),
        "8 ROMs, 126 cartridges and 7 machine configurations found."
    );
    assert_eq!(
        info(1, 1, 1).inventory(),
        "1 ROM, 1 cartridge and 1 machine configuration found."
    );
    assert_eq!(
        info(0, 0, 0).inventory(),
        "0 ROMs, 0 cartridges and 0 machine configurations found."
    );
}
