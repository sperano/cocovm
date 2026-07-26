use super::*;

/// Scratch directory under `target/` holding only the ROM files a given
/// test writes into it — deliberately not the real workspace `roms/`
/// (whose contents vary machine-to-machine), so [`compose_coco12_rom`]'s
/// candidate-preference logic is exercised deterministically.
fn scratch_roms_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp-test-roms")
        .join(name);
    std::fs::create_dir_all(&dir).expect("create scratch roms dir");
    dir
}

#[test]
fn compose_coco12_rom_lays_out_extbas_then_bas() {
    let dir = scratch_roms_dir("compose_with_extbas");
    let bas = vec![0xAAu8; COCO12_BAS_OFFSET];
    let extbas = vec![0xBBu8; COCO12_BAS_OFFSET];
    std::fs::write(dir.join("bas12.rom"), &bas).unwrap();
    std::fs::write(dir.join("extbas11.rom"), &extbas).unwrap();

    let Coco12RomResult::Composed { image, .. } = compose_coco12_rom(&dir) else {
        panic!("expected Composed");
    };
    assert_eq!(image.len(), COCO12_BAS_OFFSET * 2);
    assert_eq!(&image[..COCO12_BAS_OFFSET], &extbas[..]);
    assert_eq!(&image[COCO12_BAS_OFFSET..], &bas[..]);
}

#[test]
fn compose_coco12_rom_fills_open_bus_when_extbas_missing() {
    let dir = scratch_roms_dir("compose_without_extbas");
    let bas = vec![0xAAu8; COCO12_BAS_OFFSET];
    std::fs::write(dir.join("bas12.rom"), &bas).unwrap();

    let Coco12RomResult::Composed { image, extbas, .. } = compose_coco12_rom(&dir) else {
        panic!("expected Composed");
    };
    assert!(extbas.is_none());
    assert!(
        image[..COCO12_BAS_OFFSET]
            .iter()
            .all(|&b| b == OPEN_BUS_FILLER)
    );
    assert_eq!(&image[COCO12_BAS_OFFSET..], &bas[..]);
}

#[test]
fn compose_coco12_rom_prefers_newest_candidate_present() {
    let dir = scratch_roms_dir("compose_prefers_newest");
    // bas10 and bas12 both present: bas12 (newer) must win.
    std::fs::write(dir.join("bas10.rom"), vec![0x10u8; COCO12_BAS_OFFSET]).unwrap();
    std::fs::write(dir.join("bas12.rom"), vec![0x12u8; COCO12_BAS_OFFSET]).unwrap();

    let Coco12RomResult::Composed { bas, .. } = compose_coco12_rom(&dir) else {
        panic!("expected Composed");
    };
    assert_eq!(bas.0.file_name().unwrap(), "bas12.rom");
}

#[test]
fn compose_coco12_rom_demotes_coco2b_bas13_to_last_resort() {
    let dir = scratch_roms_dir("compose_demotes_bas13");
    // bas13 pairs with the unmodeled MC6847T1 (CoCo 2B): bas12 must win
    // over it despite being the older version number.
    std::fs::write(dir.join("bas13.rom"), vec![0x13u8; COCO12_BAS_OFFSET]).unwrap();
    std::fs::write(dir.join("bas12.rom"), vec![0x12u8; COCO12_BAS_OFFSET]).unwrap();

    let Coco12RomResult::Composed { bas, .. } = compose_coco12_rom(&dir) else {
        panic!("expected Composed");
    };
    assert_eq!(bas.0.file_name().unwrap(), "bas12.rom");
}

#[test]
fn compose_coco12_rom_reports_missing_color_basic() {
    let dir = scratch_roms_dir("compose_no_bas");
    // Directory exists but has no candidate ROMs in it.
    assert!(matches!(
        compose_coco12_rom(&dir),
        Coco12RomResult::NoColorBasic
    ));
}
