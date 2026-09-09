use coco_core::MonitorType;

use super::*;

/// Every config the form can produce must pass core validation — the choice lists,
/// `constrain`, and `Display::to_monitor` exist precisely to guarantee this.
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
                // `config.monitor`, so the monitor axis is exactly its
                // per-variant choice list.
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

/// Switching model away from CoCo 3 must snap GIME-only RAM and PAL back to plain-SAM-valid
/// values (and vice versa), and re-seed the VDG family default.
/// `constrain` deliberately doesn't touch `config.monitor` — the form's
/// Display pick owns that.
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

/// The display half of the re-constrain (run before drawing): a monitor pick snaps to the
/// default TV when the model loses its monitor port, and the config's signal path follows.
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

    // A TV pick survives the trip back to CoCo 3, resolving to the
    // composite path, not the RGB default.
    form.config.variant = MachineVariant::Coco3;
    constrain(&mut form.config);
    form.constrain_display();
    assert_eq!(form.display, Display::TV(crate::display::TV::Color));
    assert_eq!(form.config.monitor, Some(MonitorType::Composite));
    assert!(form.config.validate().is_ok());
}

#[test]
fn serial_choices_preserve_models_through_form_and_saved_definition() {
    use crate::machine_def::{self, MachineDef, tests::TempDir};
    let dir = TempDir::new("serial-models");
    for (choice, persisted) in [
        (SerialChoice::None, None),
        (SerialChoice::Printer, Some("printer")),
        (SerialChoice::Dmp130, Some("dmp130")),
        (SerialChoice::PrintFile, Some("file")),
    ] {
        let mut def = MachineDef::from_config("Serial".into(), None, &MachineConfig::default());
        def.ports.serial = choice.into();
        machine_def::save(dir.path(), "serial", &def).unwrap();
        let text = std::fs::read_to_string(dir.path().join("serial.toml")).unwrap();
        if let Some(value) = persisted {
            assert!(text.contains(&format!("serial = \"{value}\"")));
        } else {
            assert!(!text.contains("serial ="));
        }
        let loaded = machine_def::load_all(dir.path()).unwrap();
        assert_eq!(SerialChoice::from(loaded[0].1.ports.serial), choice);
    }
}
