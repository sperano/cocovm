use super::*;

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
