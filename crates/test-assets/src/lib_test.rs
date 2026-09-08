//! Unit tests for XDG-only asset path resolution.
use super::*;

#[test]
fn roms_dir_is_under_the_xdg_assets_dir() {
    assert_eq!(
        roms_dir(),
        xdg_data_dir().unwrap().join(ASSETS_KIND).join(ROMS_KIND)
    );
}

#[test]
fn tests_dir_is_under_the_xdg_assets_dir() {
    assert_eq!(
        tests_dir(),
        xdg_data_dir().unwrap().join(ASSETS_KIND).join(TESTS_KIND)
    );
}

#[test]
fn rom_path_is_under_xdg_roms_dir() {
    assert_eq!(rom(rom::COCO3), roms_dir().join(rom::COCO3));
}

#[test]
fn disk_path_is_under_xdg_tests_dir() {
    assert_eq!(disk_path(disk::EOU_BOOT), tests_dir().join(disk::EOU_BOOT));
}
