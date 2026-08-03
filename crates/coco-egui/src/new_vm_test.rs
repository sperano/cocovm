use coco_core::MonitorType;

use super::*;

/// Every config the form can produce must pass core validation — the
/// choice lists, `constrain`, and `Display::to_monitor` exist precisely to
/// guarantee this.
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
        for &memory in ram_choices(variant) {
            for &video in videos {
                // The form's Display row is the only writer of
                // `config.monitor` (`MachineForm::display_rows`), so the
                // monitor axis is exactly its per-variant choice list.
                for &display in Display::choices(variant) {
                    for &vdg in vdgs {
                        let config = MachineConfig {
                            variant,
                            video,
                            memory,
                            monitor: display.to_monitor(variant),
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
/// CoCo 1, none on CoCo 3). `config.monitor` is deliberately not
/// `constrain`'s to touch — the form's Display pick owns it
/// (`MachineForm::display_rows`'s re-constrain).
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

    config.variant = MachineVariant::Coco3;
    constrain(&mut config);
    assert_eq!(config.memory, MemorySize::K512);
    assert_eq!(config.vdg, None);

    config.variant = MachineVariant::Coco1;
    constrain(&mut config);
    assert_eq!(config.vdg, Some(VDGVariant::MC6847));
}

/// The display half of the re-constrain (`MachineForm::display_rows` runs
/// it before drawing): a monitor pick snaps to the default TV when the
/// model loses its monitor port, and the config's signal path follows.
#[test]
fn constrain_display_snaps_to_tv_where_no_monitor_port_exists() {
    let mut form = MachineForm::new("test");
    assert_eq!(form.display, Display::Monitor(MonitorType::RGB));

    form.config.variant = MachineVariant::Coco2;
    constrain(&mut form.config);
    form.constrain_display();
    assert_eq!(form.display, Display::TV(crate::display::TV::Color));
    assert_eq!(
        form.config.monitor, None,
        "a CoCo 2 has no monitor port to configure"
    );
    assert!(form.config.validate().is_ok());

    // A TV pick survives the trip back to CoCo 3 — and resolves to the
    // composite path, not the RGB default.
    form.config.variant = MachineVariant::Coco3;
    constrain(&mut form.config);
    form.constrain_display();
    assert_eq!(form.display, Display::TV(crate::display::TV::Color));
    assert_eq!(form.config.monitor, Some(MonitorType::Composite));
    assert!(form.config.validate().is_ok());
}
