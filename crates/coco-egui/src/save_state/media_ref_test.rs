use coco_core::snapshot::CartROMRole;

use crate::rom_load::COCO3_ROM_FILE;
use crate::{AppParams, CocoApp, ROMSource, installed_roms_dir, rom_load};
use coco_core::MachineConfig;

/// A booted app with a Multi-Pak; `None` when either SSC image is missing.
fn app_with_ssc_in_slot(slot: usize) -> Option<CocoApp> {
    let roms_dir = installed_roms_dir();
    if !roms_dir.join(rom_load::SSC_FIRMWARE_ROM).is_file()
        || !roms_dir.join(rom_load::SP0256_ROM).is_file()
    {
        return None;
    }
    let rom_path = roms_dir.join(COCO3_ROM_FILE);
    let rom = std::fs::read(&rom_path)
        .expect("installed coco3.rom is required (first-run asset download)")
        .into_boxed_slice();
    let mut app = CocoApp::new(
        MachineConfig::default(),
        rom,
        ROMSource::File(rom_path),
        AppParams::default(),
        crate::joy::SharedGamepad::without_backend(),
    );
    app.insert_multipak();
    app.mpi_insert_ssc(slot);
    assert!(app.cart_error.is_none(), "{:?}", app.cart_error);
    Some(app)
}

/// Both SSC images are recorded for the slot, each under its own role and
/// pointing at the installed file; a swap would fail every SSC restore.
#[test]
fn media_refs_record_the_ssc_speech_rom_and_firmware_by_role() {
    const SLOT: usize = 1;
    let Some(mut app) = app_with_ssc_in_slot(SLOT) else {
        eprintln!(
            "skipping media_refs_record_the_ssc_speech_rom_and_firmware_by_role: SSC ROMs not present"
        );
        return;
    };
    let media = app.build_media_refs().expect("build_media_refs");
    let slot_ref = |role: CartROMRole| {
        media
            .cart_roms
            .iter()
            .find(|r| r.mpi_slot == Some(SLOT as u8) && r.role == role)
            .unwrap_or_else(|| panic!("{role:?} image for slot {SLOT} must be recorded"))
    };
    assert_eq!(
        slot_ref(CartROMRole::Primary).rom.path,
        rom_load::sp0256_rom_path()
    );
    assert_eq!(
        slot_ref(CartROMRole::SSCFirmware).rom.path,
        rom_load::ssc_firmware_rom_path()
    );
    assert_eq!(
        media
            .cart_roms
            .iter()
            .filter(|r| r.mpi_slot == Some(SLOT as u8))
            .count(),
        2,
        "exactly the two SSC images"
    );
}
