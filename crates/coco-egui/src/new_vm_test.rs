use super::*;

/// Every config the form can produce must pass core validation — the
/// choice lists and `constrain` exist precisely to guarantee this.
#[test]
fn every_selectable_config_validates() {
    for variant in MachineVariant::ALL {
        let videos: &[VideoStandard] = if variant == MachineVariant::Coco3 {
            &[VideoStandard::NTSC, VideoStandard::PAL]
        } else {
            &[VideoStandard::NTSC]
        };
        let vdgs: &[Option<VDGVariant>] = match variant {
            MachineVariant::Coco2 => &[Some(VDGVariant::MC6847), Some(VDGVariant::MC6847T1)],
            MachineVariant::Coco1 => &[Some(VDGVariant::MC6847)],
            MachineVariant::Coco3 => &[None],
        };
        let monitors: &[Option<MonitorType>] = if variant == MachineVariant::Coco3 {
            &[Some(MonitorType::RGB), Some(MonitorType::Composite)]
        } else {
            &[None]
        };
        for &memory in ram_choices(variant) {
            for &video in videos {
                for &monitor in monitors {
                    for &vdg in vdgs {
                        let config = MachineConfig {
                            variant,
                            video,
                            memory,
                            monitor,
                            vdg,
                        };
                        assert!(
                            config.validate().is_ok(),
                            "form offered invalid config: {config:?}"
                        );
                    }
                }
            }
        }
    }
}

/// Switching model away from CoCo 3 must snap GIME-only RAM and PAL back
/// to plain-SAM-valid values (and vice versa for RAM); switching models
/// re-seeds the VDG family default (T1 on CoCo 2, plain MC6847 on
/// CoCo 1, none on CoCo 3) and the monitor (a cable choice only where a
/// monitor port exists — the CoCo 3).
#[test]
fn constrain_draft_snaps_family_specific_fields() {
    let mut config = MachineConfig {
        variant: MachineVariant::Coco3,
        video: VideoStandard::PAL,
        memory: MemorySize::K2048,
        monitor: Some(MonitorType::Composite),
        vdg: None,
    };

    config.variant = MachineVariant::Coco2;
    constrain(&mut config);
    assert_eq!(config.memory, MemorySize::K64);
    assert_eq!(config.video, VideoStandard::NTSC);
    assert_eq!(
        config.vdg,
        Some(VDGVariant::MC6847T1),
        "CoCo 2 defaults to the T1 (CoCo 2B)"
    );
    assert_eq!(
        config.monitor, None,
        "a CoCo 2 has no monitor port to configure"
    );
    assert!(config.validate().is_ok());

    config.variant = MachineVariant::Coco3;
    constrain(&mut config);
    assert_eq!(config.memory, MemorySize::K512);
    assert_eq!(config.vdg, None);
    assert_eq!(
        config.monitor,
        Some(MonitorType::RGB),
        "returning to CoCo 3 re-seeds the default cable"
    );
    assert!(config.validate().is_ok());

    config.variant = MachineVariant::Coco1;
    constrain(&mut config);
    assert_eq!(config.vdg, Some(VDGVariant::MC6847));
    assert_eq!(config.monitor, None);
    assert!(config.validate().is_ok());
}
