//! Unit tests for XDG-only asset path resolution.
use super::*;

#[test]
fn roms_dir_is_under_xdg_data_dir() {
    assert_eq!(roms_dir(), xdg_data_dir().unwrap().join(ROMS_KIND));
}

#[test]
fn disks_dir_is_under_xdg_data_dir() {
    assert_eq!(disks_dir(), xdg_data_dir().unwrap().join(DISKS_KIND));
}

#[test]
fn rom_path_is_under_xdg_roms_dir() {
    assert_eq!(rom(rom::COCO3), roms_dir().join(rom::COCO3));
}

#[test]
fn disk_path_is_under_xdg_disks_dir() {
    assert_eq!(disk(disk::EOU_BOOT), disks_dir().join(disk::EOU_BOOT));
}
