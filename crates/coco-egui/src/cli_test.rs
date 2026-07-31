use super::*;

#[test]
fn parse_machine_accepts_known_spellings_and_rejects_others() {
    assert_eq!(parse_machine("coco1"), Ok(MachineVariant::Coco1));
    assert_eq!(parse_machine("coco2"), Ok(MachineVariant::Coco2));
    assert_eq!(parse_machine("coco3"), Ok(MachineVariant::Coco3));
    assert!(parse_machine("coco4").is_err());
    assert!(parse_machine("").is_err());
}

#[test]
fn parse_ram_accepts_every_memory_size_spelling() {
    assert_eq!(parse_ram("4k"), Ok(MemorySize::K4));
    assert_eq!(parse_ram("16k"), Ok(MemorySize::K16));
    assert_eq!(parse_ram("32k"), Ok(MemorySize::K32));
    assert_eq!(parse_ram("64k"), Ok(MemorySize::K64));
    assert_eq!(parse_ram("128k"), Ok(MemorySize::K128));
    assert_eq!(parse_ram("512k"), Ok(MemorySize::K512));
    assert_eq!(parse_ram("2048k"), Ok(MemorySize::K2048));
    assert!(parse_ram("1mb").is_err());
}

#[test]
fn parse_video_accepts_ntsc_and_pal() {
    assert_eq!(parse_video("ntsc"), Ok(VideoStandard::NTSC));
    assert_eq!(parse_video("pal"), Ok(VideoStandard::PAL));
    assert!(parse_video("secam").is_err());
}

#[test]
fn default_ram_is_512k_for_coco3_and_64k_for_coco1_2() {
    assert_eq!(default_ram(MachineVariant::Coco3), MemorySize::K512);
    assert_eq!(default_ram(MachineVariant::Coco1), MemorySize::K64);
    assert_eq!(default_ram(MachineVariant::Coco2), MemorySize::K64);
}

#[test]
fn default_vdg_is_t1_for_coco2_and_plain_elsewhere() {
    assert_eq!(
        default_vdg(MachineVariant::Coco2),
        Some(VDGVariant::MC6847T1)
    );
    assert_eq!(default_vdg(MachineVariant::Coco1), Some(VDGVariant::MC6847));
    assert_eq!(default_vdg(MachineVariant::Coco3), None);
}
