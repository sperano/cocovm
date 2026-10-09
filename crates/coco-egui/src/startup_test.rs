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
fn missing_bundled_cartridges_names_only_the_absent_files() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-assets/missing-cartridges");
    let _ = std::fs::remove_dir_all(&dir);
    let all: Vec<&str> = bundled_cartridges().collect();
    assert!(!all.is_empty());
    assert_eq!(missing_bundled_cartridges(&dir), all, "no dir: all missing");

    std::fs::create_dir_all(&dir).unwrap();
    for name in &all {
        std::fs::write(dir.join(name), b"\xAA").unwrap();
    }
    assert!(missing_bundled_cartridges(&dir).is_empty());

    let removed = all[0];
    std::fs::remove_file(dir.join(removed)).unwrap();
    assert_eq!(missing_bundled_cartridges(&dir), vec![removed]);
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

#[test]
fn visible_width_ignores_ansi_color() {
    assert_eq!(visible_width("\x1b[2m│\x1b[0m"), 1);
    assert_eq!(visible_width("\x1b[38;5;209m/\x1b[39m © É"), 5);
}

#[test]
fn wrap_breaks_between_words_within_width() {
    assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
    assert_eq!(wrap("aa   bb", 10), ["aa bb"]);
    assert_eq!(wrap("", 10), [""]);
}

#[test]
fn wrap_splits_a_word_wider_than_the_line() {
    assert_eq!(wrap("x abcdefgh", 3), ["x", "abc", "def", "gh"]);
}

#[test]
fn wrap_measures_double_width_characters() {
    // Each CJK ideograph takes two terminal columns.
    assert_eq!(wrap("漢字漢字", 4), ["漢字", "漢字"]);
}

/// Columns the box's two walls add around its inner width.
const WALL_COLUMNS: usize = 2;

/// The renderer line from the report, wider than the box.
const LONG_RENDERER: &str = "OpenGL version: 3.3 INTEL-24.1.11 (Parallels using \
    Intel(R) Iris(TM) Plus Graphics OpenGL Engine (Compat)).";

#[test]
fn banner_wraps_long_rows_inside_the_walls() {
    let title = banner_title();
    let lines = banner_lines(&title, &[LONG_RENDERER, "17 ROMs found."]);
    let box_width = BANNER_WIDTH + WALL_COLUMNS;
    for line in &lines {
        assert_eq!(visible_width(line), box_width, "{line:?}");
    }
    // Frame, title, rule, at least two renderer lines, inventory, frame.
    const MIN_LINES: usize = 7;
    assert!(lines.len() >= MIN_LINES, "{lines:#?}");
    // Top frame, title and rule above the rows; bottom frame below.
    const HEADER_LINES: usize = 3;
    let rows: Vec<String> = lines[HEADER_LINES..lines.len() - 1]
        .iter()
        .map(|line| strip_ansi(line))
        .collect();
    let body: Vec<&str> = rows
        .iter()
        .flat_map(|row| row.trim_matches(|c| c == '│' || c == ' ').split(' '))
        .filter(|word| !word.is_empty())
        .collect();
    let expected: Vec<&str> = LONG_RENDERER
        .split_whitespace()
        .chain("17 ROMs found.".split(' '))
        .collect();
    assert_eq!(body, expected);
}

#[test]
fn banner_widens_for_a_title_longer_than_the_default() {
    let title = "t".repeat(BANNER_WIDTH);
    let lines = banner_lines(&title, &["short"]);
    let box_width = BANNER_WIDTH + 2 * BANNER_MARGIN + WALL_COLUMNS;
    for line in &lines {
        assert_eq!(visible_width(line), box_width, "{line:?}");
    }
}
