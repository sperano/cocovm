use super::*;

#[test]
fn default_config_is_valid() {
    assert!(MachineConfig::default().validate().is_ok());
}

#[test]
fn coco3_rejects_coco12_memory_sizes() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco3,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: Some(MonitorType::RGB),
        vdg: None,
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn coco1_rejects_coco3_memory_sizes() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K128,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn coco1_accepts_every_plain_sam_memory_size() {
    for memory in [
        MemorySize::K4,
        MemorySize::K16,
        MemorySize::K32,
        MemorySize::K64,
    ] {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco1,
            video: VideoStandard::NTSC,
            memory,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
        };
        assert!(
            cfg.validate().is_ok(),
            "{memory:?} should be valid for Coco1"
        );
    }
}

#[test]
fn coco2_only_accepts_shipped_memory_sizes() {
    for (memory, ok) in [
        (MemorySize::K4, false),
        (MemorySize::K16, true),
        (MemorySize::K32, false),
        (MemorySize::K64, true),
    ] {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco2,
            video: VideoStandard::NTSC,
            memory,
            monitor: None,
            vdg: Some(VDGVariant::MC6847),
        };
        assert_eq!(
            cfg.validate().is_ok(),
            ok,
            "{memory:?} for Coco2: expected valid={ok}"
        );
    }
}

#[test]
fn coco2_rejects_pal() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::PAL,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn coco3_accepts_pal() {
    // PAL is only out of scope for the plain-SAM variants; the GIME path
    // already models a (partially unverified) PAL timing branch.
    let cfg = MachineConfig {
        variant: MachineVariant::Coco3,
        video: VideoStandard::PAL,
        memory: MemorySize::K512,
        monitor: Some(MonitorType::RGB),
        vdg: None,
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn coco2_accepts_mc6847t1() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847T1),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn coco2_accepts_plain_mc6847() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn coco1_rejects_mc6847t1() {
    let cfg = MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: None,
        vdg: Some(VDGVariant::MC6847T1),
    };
    assert!(cfg.validate().is_err());
}

#[test]
fn coco3_rejects_any_vdg() {
    for vdg in [VDGVariant::MC6847, VDGVariant::MC6847T1] {
        let cfg = MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::NTSC,
            memory: MemorySize::K512,
            monitor: Some(MonitorType::RGB),
            vdg: Some(vdg),
        };
        assert!(cfg.validate().is_err(), "Coco3 has no VDG, {vdg:?} must be rejected");
    }
}

#[test]
fn coco12_rejects_monitor_and_coco3_requires_one() {
    let mut cfg = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K64,
        monitor: Some(MonitorType::RGB),
        vdg: Some(VDGVariant::MC6847),
    };
    assert!(cfg.validate().is_err(), "Coco2 has no monitor port");
    cfg.monitor = None;
    assert!(cfg.validate().is_ok());

    let mut cfg = MachineConfig::default();
    assert!(cfg.validate().is_ok());
    cfg.monitor = None;
    assert!(cfg.validate().is_err(), "Coco3 needs a cable choice");
}

#[test]
fn fs_falling_line_differs_between_gime_and_plain_vdg() {
    assert_eq!(
        VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco3),
        244
    );
    assert_eq!(
        VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco1),
        216
    );
    assert_eq!(
        VideoStandard::NTSC.fs_falling_line(MachineVariant::Coco2),
        216
    );
}
