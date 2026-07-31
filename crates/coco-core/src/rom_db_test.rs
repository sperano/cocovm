use super::*;

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
