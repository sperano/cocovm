use super::*;

fn bytes_with_crc_suffix(size: usize, suffix: [u8; 4]) -> Vec<u8> {
    const CRC_SUFFIX_SIZE: usize = 4;
    let mut bytes = vec![0; size - CRC_SUFFIX_SIZE];
    bytes.extend_from_slice(&suffix);
    bytes
}

#[test]
fn crc32_check_value() {
    // The standard CRC-32/ISO-HDLC check value.
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn manifest_has_no_duplicate_names_or_crcs() {
    for (i, a) in KNOWN_ROMS.iter().enumerate() {
        for b in &KNOWN_ROMS[i + 1..] {
            assert_ne!(a.file, b.file);
            assert_ne!(
                (a.crc32, a.size),
                (b.crc32, b.size),
                "{} vs {}",
                a.file,
                b.file
            );
        }
    }
}

#[test]
fn system_roms_are_tagged_with_their_machine() {
    let family = |file: &str| {
        KNOWN_ROMS
            .iter()
            .find(|rom| rom.file == file)
            .unwrap_or_else(|| panic!("{file} not in KNOWN_ROMS"))
            .machine
    };
    assert_eq!(family("coco3.rom"), MachineFamily::Coco3);
    assert_eq!(family("bas12.rom"), MachineFamily::Coco12);
    assert_eq!(family("extbas11.rom"), MachineFamily::Coco12);
    assert_eq!(family("disk11.rom"), MachineFamily::Any);
    assert_eq!(family("hdbdw3bck.rom"), MachineFamily::Coco12);
    assert_eq!(family("hdbdw3bc3.rom"), MachineFamily::Coco3);
}

#[test]
fn validate_flags_corrupt_known_name() {
    let bytes = vec![0u8; 0x2000];
    match validate("bas12.rom", &bytes) {
        Validation::Mismatch {
            expected,
            actual_size,
            ..
        } => {
            assert_eq!(expected.file, "bas12.rom");
            assert_eq!(actual_size, 0x2000);
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
}

#[test]
fn validate_passes_unknown_names_through() {
    assert_eq!(validate("homebrew.rom", &[0u8; 16]), Validation::Unknown);
}

#[test]
fn cartridge_manifest_has_expected_counts() {
    const XROAR_COCO_CARTRIDGES: usize = 101;
    const COCOVM_ADDITIONS: usize = 1;
    const BANKED_ROM_PAKS: usize = 4;
    const GMC_CARTRIDGES: usize = 3;
    const COCO3_ONLY: usize = 14;
    const COCO12_ONLY: usize = 2;

    assert_eq!(
        KNOWN_CARTRIDGE_ROMS.len(),
        XROAR_COCO_CARTRIDGES + COCOVM_ADDITIONS
    );
    assert_eq!(
        KNOWN_CARTRIDGE_ROMS
            .iter()
            .filter(|rom| rom.hardware == CartridgeHardware::BankedRomPak)
            .count(),
        BANKED_ROM_PAKS
    );
    assert_eq!(
        KNOWN_CARTRIDGE_ROMS
            .iter()
            .filter(|rom| rom.hardware == CartridgeHardware::GamesMaster)
            .count(),
        GMC_CARTRIDGES
    );
    assert_eq!(
        KNOWN_CARTRIDGE_ROMS
            .iter()
            .filter(|rom| rom.machine == MachineFamily::Coco3)
            .count(),
        COCO3_ONLY
    );
    assert_eq!(
        KNOWN_CARTRIDGE_ROMS
            .iter()
            .filter(|rom| rom.machine == MachineFamily::Coco12)
            .count(),
        COCO12_ONLY
    );
}

#[test]
fn cartridge_machine_supports_matches_family() {
    use crate::config::MachineVariant;
    for variant in MachineVariant::ALL {
        assert!(MachineFamily::Any.supports(variant), "{variant:?}");
        assert_eq!(
            MachineFamily::Coco3.supports(variant),
            variant == MachineVariant::Coco3,
            "{variant:?}"
        );
        assert_eq!(
            MachineFamily::Coco12.supports(variant),
            variant != MachineVariant::Coco3,
            "{variant:?}"
        );
    }
}

#[test]
fn coco3_only_titles_are_tagged() {
    let coco3: Vec<&str> = KNOWN_CARTRIDGE_ROMS
        .iter()
        .filter(|rom| rom.machine == MachineFamily::Coco3)
        .map(|rom| rom.name)
        .collect();
    for title in ["Thexder", "Predator", "RoboCop", "Castle of Tharoggad"] {
        assert!(coco3.contains(&title), "{title}");
    }
    let daggorath = KNOWN_CARTRIDGE_ROMS
        .iter()
        .find(|rom| rom.name == "Dungeons of Daggorath")
        .unwrap();
    assert_eq!(daggorath.machine, MachineFamily::Any);
}

#[test]
fn cartridge_manifest_has_no_duplicate_fingerprints() {
    for (index, first) in KNOWN_CARTRIDGE_ROMS.iter().enumerate() {
        for second in &KNOWN_CARTRIDGE_ROMS[index + 1..] {
            assert_ne!(
                (first.crc32, first.size),
                (second.crc32, second.size),
                "{} vs {}",
                first.title(),
                second.title()
            );
        }
    }
}

#[test]
fn cartridge_manifest_bundled_files_are_unique_ccc_names() {
    const BUNDLED_CARTRIDGES: usize = 89;
    let bundled: Vec<&str> = KNOWN_CARTRIDGE_ROMS
        .iter()
        .filter_map(|rom| rom.bundled_file)
        .collect();
    assert_eq!(bundled.len(), BUNDLED_CARTRIDGES);
    for (index, file) in bundled.iter().enumerate() {
        assert!(file.ends_with(".ccc"), "{file}");
        assert!(!file.contains('/'), "{file}");
        assert!(!bundled[index + 1..].contains(file), "duplicate {file}");
    }
}

#[test]
fn identifies_known_rom_pak_content() {
    const ANDRONE_SIZE: usize = 0x2000;
    const ANDRONE_CRC_SUFFIX: [u8; 4] = [0xf2, 0x92, 0x9e, 0x42];
    let bytes = bytes_with_crc_suffix(ANDRONE_SIZE, ANDRONE_CRC_SUFFIX);

    let known = identify_cartridge(&bytes).unwrap();
    assert_eq!(known.title(), "Androne (1983) (Tandy) (26-3096)");
    assert_eq!(known.hardware, CartridgeHardware::RomPak);
}

#[test]
fn identifies_cyd_games_master_content() {
    const CYD_SIZE: usize = 0x2000;
    const CYD_CRC_SUFFIX: [u8; 4] = [0x17, 0x60, 0x50, 0x9b];
    let bytes = bytes_with_crc_suffix(CYD_SIZE, CYD_CRC_SUFFIX);

    assert_eq!(
        identify_cartridge(&bytes).map(|known| known.hardware),
        Some(CartridgeHardware::GamesMaster)
    );
}

#[test]
fn identifies_every_banked_rom_pak_fingerprint() {
    const EXPECTED_BANKED_ROM_PAKS: &[(usize, u32, &str)] = &[
        (0x8000, 0x83bd_6056, "Mind Roll (1988) (Tandy) (26-3100)"),
        (0x10000, 0xa968_0ede, "Predator (1989) (Tandy) (26-3165)"),
        (0x20000, 0xdd94_dd06, "RoboCop (1988) (Tandy) (26-3164)"),
        (
            0x8000,
            0x8789_06fe,
            "Mind Roll (1988) (Tandy) (26-3100) [f plane1]",
        ),
    ];

    for &(size, crc32, desc) in EXPECTED_BANKED_ROM_PAKS {
        let known = identify_cartridge_fingerprint(size, crc32).unwrap();
        assert_eq!(known.title(), desc);
        assert_eq!(known.hardware, CartridgeHardware::BankedRomPak);
    }
}

#[test]
fn identifies_every_games_master_fingerprint() {
    const EXPECTED_GMC: &[(usize, u32, &str)] = &[
        (0x4000, 0xabe7_bb9e, "Blockdown (2021) (Teipen Mwnci)"),
        (0x10000, 0x5871_6b7f, "Dunjunz (2020) (Teipen Mwnci)"),
        (0x2000, 0x808b_2a0a, "CyD Games Master Cartridge ROM"),
    ];

    for &(size, crc32, desc) in EXPECTED_GMC {
        let known = identify_cartridge_fingerprint(size, crc32).unwrap();
        assert_eq!(known.title(), desc);
        assert_eq!(known.hardware, CartridgeHardware::GamesMaster);
    }
}

#[test]
fn cartridge_fingerprint_rejects_wrong_size() {
    const ANDRONE_SIZE: usize = 0x2000;
    const ANDRONE_CRC32: u32 = 0x7d1c_ac0e;

    assert_eq!(
        identify_cartridge_fingerprint(ANDRONE_SIZE + 1, ANDRONE_CRC32),
        None
    );
}

#[test]
fn unknown_cartridge_content_is_not_identified() {
    assert_eq!(identify_cartridge(&[0; 16]), None);
}
