use super::*;

/// Every config the dialog can produce must pass core validation — the
/// choice lists and `constrain_draft` exist precisely to guarantee this.
#[test]
fn every_selectable_config_validates() {
    for variant in [
        MachineVariant::Coco1,
        MachineVariant::Coco2,
        MachineVariant::Coco3,
    ] {
        let videos: &[VideoStandard] = if variant == MachineVariant::Coco3 {
            &[VideoStandard::NTSC, VideoStandard::PAL]
        } else {
            &[VideoStandard::NTSC]
        };
        let vdgs: &[Option<VDGVariant>] = match variant {
            MachineVariant::Coco2 => {
                &[Some(VDGVariant::MC6847), Some(VDGVariant::MC6847T1)]
            }
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
                            "dialog offered invalid config: {config:?}"
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
    let mut dialog = NewVmDialog::new();
    dialog.open_with(
        MachineConfig {
            variant: MachineVariant::Coco3,
            video: VideoStandard::PAL,
            memory: MemorySize::K2048,
            monitor: Some(MonitorType::Composite),
            vdg: None,
        },
        true,
        crate::KbMode::Positional,
    );

    dialog.form.config.variant = MachineVariant::Coco2;
    constrain(&mut dialog.form.config);
    assert_eq!(dialog.form.config.memory, MemorySize::K64);
    assert_eq!(dialog.form.config.video, VideoStandard::NTSC);
    assert_eq!(
        dialog.form.config.vdg,
        Some(VDGVariant::MC6847T1),
        "CoCo 2 defaults to the T1 (CoCo 2B)"
    );
    assert_eq!(
        dialog.form.config.monitor, None,
        "a CoCo 2 has no monitor port to configure"
    );
    assert!(dialog.form.config.validate().is_ok());

    dialog.form.config.variant = MachineVariant::Coco3;
    constrain(&mut dialog.form.config);
    assert_eq!(dialog.form.config.memory, MemorySize::K512);
    assert_eq!(dialog.form.config.vdg, None);
    assert_eq!(
        dialog.form.config.monitor,
        Some(MonitorType::RGB),
        "returning to CoCo 3 re-seeds the default cable"
    );
    assert!(dialog.form.config.validate().is_ok());

    dialog.form.config.variant = MachineVariant::Coco1;
    constrain(&mut dialog.form.config);
    assert_eq!(dialog.form.config.vdg, Some(VDGVariant::MC6847));
    assert_eq!(dialog.form.config.monitor, None);
    assert!(dialog.form.config.validate().is_ok());
}

/// Re-opening seeds the draft from the running machine and clears any
/// stale error from a previous failed attempt.
#[test]
fn open_with_seeds_draft_and_clears_error() {
    let mut dialog = NewVmDialog::new();
    dialog.error = Some("old failure".into());
    let current = MachineConfig {
        variant: MachineVariant::Coco1,
        video: VideoStandard::NTSC,
        memory: MemorySize::K16,
        monitor: None,
        vdg: Some(VDGVariant::MC6847),
    };
    dialog.open_with(current, false, crate::KbMode::Symbolic);
    assert!(dialog.open);
    assert!(dialog.error.is_none());
    assert_eq!(dialog.form.config.variant, MachineVariant::Coco1);
    assert_eq!(dialog.form.config.memory, MemorySize::K16);
}
