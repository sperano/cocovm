use super::dto::{
    DisplayDTO, HiResInterfaceDTO, JoySourceDTO, MachineVariantDTO, RAMDTO, SerialDTO, StatsDTO,
    VDGVariantDTO, VideoStandardDTO,
};
use super::*;
use super::{CartridgeDTO, SlotDTO};
use crate::display::{TV, TVSettings};
use coco_core::MonitorType;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

/// Unique temp directory per test, cleaned up on drop so parallel tests
/// never collide and nothing lingers under the OS temp dir. `pub(crate)`
/// so `ui_tests.rs` (a sibling module, both descendants of the crate
/// root) shares this instead of keeping its own copy.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "coco-egui-machine-def-test-{tag}-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn full_def() -> MachineDef {
    MachineDef {
        schema: CURRENT_SCHEMA,
        name: "Dev CoCo 3".to_string(),
        created: Some("2026-07-16".to_string()),
        hardware: HardwareDTO {
            variant: MachineVariantDTO::Coco3,
            ram: RAMDTO::K512,
            video: VideoStandardDTO::NTSC,
            // What saves write: `display` only, never the legacy `monitor` key.
            monitor: None,
            display: Some(DisplayDTO::RGB),
            vdg: None,
            rom: Some("/path/custom.rom".to_string()),
        },
        media: MediaDTO {
            disk0: Some("dev.dsk".to_string()),
            disk1: Some("/shared/utils.dsk".to_string()),
            vhd0: Some("/vhd/68SDC.VHD".to_string()),
            vhd1: None,
            tape: Some("session.cas".to_string()),
        },
        drivewire: DriveWireDTO {
            enabled: true,
            hdbdos_mode: true,
            disk0: Some("dw0.dsk".to_string()),
            disk1: None,
            disk2: Some("/shared/dw2.dsk".to_string()),
            disk3: None,
        },
        peripherals: PeripheralsDTO {
            cartridge: CartridgeDTO::MPI {
                slots: [
                    SlotDTO::ROMPak {
                        path: "/paks/arkanoid.ccc".to_string(),
                    },
                    SlotDTO::Empty,
                    SlotDTO::RTC,
                    SlotDTO::FD502 {
                        dos_rom: Default::default(),
                    },
                ],
                switch: 2,
            },
        },
        ports: PortsDTO {
            serial: Some(SerialDTO::Printer),
        },
        ui: UIDTO {
            kb_mode: KbModeDTO::Symbolic,
            // Both away from their shared `None` default, so the round trip exercises
            // non-default values.
            joy_left: JoySourceDTO::Keys,
            joy_right: JoySourceDTO::Gamepad,
            // Only `hires_right` moves off the shared `None` default: only one port can ever
            // have Tandy installed (one physical DAC), so `hires_left` staying `None` here is
            // the only valid pairing that also exercises the non-default value.
            hires_left: HiResInterfaceDTO::None,
            hires_right: HiResInterfaceDTO::Tandy,
            // Away from the defaults (35/5/5) so the round trip exercises all TV settings.
            tv_scanline: 60,
            tv_noise: 20,
            tv_overscan: 7,
        },
        // Away from the defaults (0/0) so the round trip exercises non-default stats.
        stats: StatsDTO {
            runtime_secs: 12_345,
            starts: 7,
        },
        unknown: toml::Table::new(),
    }
}

/// A new enum variant's serde shape is easy to get wrong silently ([`HiResInterfaceDTO`]'s
/// `rename_all = "lowercase"` relies on the derive lowercasing `CoCoMax3` verbatim, not a
/// manual rename) — confirmed directly rather than only via [`full_def`]'s one sampled value.
#[test]
fn hires_interface_dto_cocomax3_serializes_lowercase() {
    let value = toml::Value::try_from(HiResInterfaceDTO::CoCoMax3).expect("serialize");
    assert_eq!(value.as_str(), Some("cocomax3"));
}

#[test]
fn round_trip_full_definition() {
    let dir = TempDir::new("roundtrip");
    let def = full_def();
    save(dir.path(), "dev-coco-3", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("every file is valid");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].0, "dev-coco-3");
    assert_eq!(&loaded[0].1, &def);
}

#[test]
fn missing_drivewire_section_defaults_disabled() {
    let parsed: MachineDef = toml::from_str(
        r#"
schema = 1
name = "Legacy"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
"#,
    )
    .expect("definition without DriveWire should parse");

    assert_eq!(parsed.drivewire, DriveWireDTO::default());
}

/// `[peripherals].cartridge = { kind = "rs232" }` and `[ports].serial`
/// round-trip through save/load; this covers the `rs232` cartridge kind and
/// the other `serial` variant, `"file"`.
#[test]
fn rs232_and_serial_file_round_trip() {
    let dir = TempDir::new("rs232-roundtrip");
    let mut def = full_def();
    def.peripherals = PeripheralsDTO {
        cartridge: CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::Loopback,
        },
    };
    def.ports.serial = Some(SerialDTO::File);
    save(dir.path(), "rs232", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("every file is valid");
    assert_eq!(loaded.len(), 1);
    assert_eq!(&loaded[0].1, &def);

    let contents = fs::read_to_string(dir.path().join("rs232.toml")).unwrap();
    assert!(
        contents.contains("kind = \"rs232\""),
        "TOML must record the rs232 cartridge:\n{contents}"
    );
    assert!(
        contents.contains("serial = \"file\""),
        "TOML must record the serial port's sink:\n{contents}"
    );
}

/// `[ui].joy_left`/`joy_right` round-trip like every other `[ui]` field;
/// this checks the TOML text itself records `"keys"`/`"gamepad"`.
#[test]
fn joy_sources_round_trip() {
    let dir = TempDir::new("joy-roundtrip");
    let def = full_def();
    save(dir.path(), "joy", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("every file is valid");
    assert_eq!(loaded.len(), 1);
    assert_eq!(&loaded[0].1, &def);

    let contents = fs::read_to_string(dir.path().join("joy.toml")).unwrap();
    assert!(
        contents.contains("joy_left = \"keys\""),
        "TOML must record the left joystick source:\n{contents}"
    );
    assert!(
        contents.contains("joy_right = \"gamepad\""),
        "TOML must record the right joystick source:\n{contents}"
    );
}

/// A default `[ports].serial` (`None`) is left out of the written TOML
/// entirely — serde skips `None` `Option` fields — while the `[ports]` table
/// header itself is still always written, even empty.
#[test]
fn default_ports_omits_serial_key_on_save() {
    let dir = TempDir::new("default-ports-save");
    let mut def = full_def();
    def.ports.serial = None;
    save(dir.path(), "no-serial", &def).expect("save should succeed");

    let contents = fs::read_to_string(dir.path().join("no-serial.toml")).unwrap();
    assert!(
        !contents.contains("serial"),
        "a default [ports].serial must not be written:\n{contents}"
    );
}

#[test]
fn minimal_file_uses_defaults() {
    let dir = TempDir::new("minimal");
    let toml_text = r#"
schema = 1
name = "Bare"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
"#;
    fs::write(dir.path().join("bare.toml"), toml_text).unwrap();

    let loaded = load_all(dir.path()).expect("minimal file should parse");
    assert_eq!(loaded.len(), 1);
    let def = &loaded[0].1;
    assert_eq!(def.created, None);
    assert_eq!(def.hardware.vdg, None);
    assert_eq!(def.hardware.rom, None);
    assert_eq!(def.media, MediaDTO::default());
    assert_eq!(def.peripherals.cartridge, CartridgeDTO::None);
    assert_eq!(def.ports.serial, None);
    assert_eq!(def.ui.kb_mode, KbModeDTO::Positional);
    // Absent joy_left/joy_right ⇒ same defaults `JoystickInputs::new` boots with: off until
    // opted in.
    assert_eq!(def.ui.joy_left, JoySourceDTO::None);
    assert_eq!(def.ui.joy_right, JoySourceDTO::None);
    assert_eq!(
        def.ui.tv_overscan,
        TVSettings::default().overscan_pct,
        "an absent tv_overscan key uses the CRT-like default"
    );

    // Default VDG is per-variant: a CoCo 3 has none at all.
    let config = def.to_machine_config().expect("should validate");
    assert_eq!(config.vdg, None);

    // The legacy `monitor` key (no `display`) maps to the monitor half of `Display`.
    assert_eq!(def.display(), Display::Monitor(MonitorType::RGB));
}

/// A file with no `[stats]` section loads with a zeroed [`StatsDTO`] and no
/// unknown-key warning (`stats` is a known section).
#[test]
fn missing_stats_section_defaults_to_zero() {
    let dir = TempDir::new("missing-stats");
    let toml_text = r#"
schema = 1
name = "No Stats Yet"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
"#;
    fs::write(dir.path().join("no-stats.toml"), toml_text).unwrap();

    let loaded = load_all(dir.path()).expect("a file with no [stats] section should still load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].1.stats, StatsDTO::default());
}

#[test]
fn minimal_coco2_defaults_to_t1_vdg() {
    let dir = TempDir::new("minimal-coco2");
    let toml_text = r#"
schema = 1
name = "Bare CoCo 2"

[hardware]
variant = "coco2"
ram = "64k"
video = "ntsc"
"#;
    fs::write(dir.path().join("bare2.toml"), toml_text).unwrap();
    let loaded = load_all(dir.path()).expect("minimal file should parse");
    let config = loaded[0].1.to_machine_config().expect("should validate");
    assert_eq!(config.vdg, Some(VDGVariant::MC6847T1));
    assert_eq!(config.monitor, None, "no monitor key, no monitor port");
    assert_eq!(
        loaded[0].1.display(),
        Display::Monitor(MonitorType::Composite),
        "a CoCo 1/2's default display is the crisp composite monitor"
    );
}

/// `[hardware].display = "tv-bw"` round-trips and resolves to the composite
/// signal path on a CoCo 3 (the TV hangs off the RF modulator).
#[test]
fn display_tv_bw_round_trips_and_forces_composite() {
    let dir = TempDir::new("display-tv-bw");
    let mut def = full_def();
    def.hardware.display = Some(DisplayDTO::TVBW);
    save(dir.path(), "bw", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("every file is valid");
    assert_eq!(&loaded[0].1, &def);
    assert_eq!(loaded[0].1.display(), Display::TV(TV::BW));
    let config = loaded[0].1.to_machine_config().expect("should validate");
    assert_eq!(config.monitor, Some(MonitorType::Composite));

    let contents = fs::read_to_string(dir.path().join("bw.toml")).unwrap();
    assert!(
        contents.contains("display = \"tv-bw\""),
        "TOML must record the display:\n{contents}"
    );
}

/// `display` supersedes the legacy `monitor` key: it wins when both are
/// present, and a legacy-only file re-saves with `display` instead.
#[test]
fn display_supersedes_the_legacy_monitor_key() {
    let dir = TempDir::new("display-legacy");
    fs::write(
        dir.path().join("legacy.toml"),
        r#"
schema = 1
name = "Legacy"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "composite"
display = "tv"
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("legacy monitor key must still load");
    let def = &loaded[0].1;
    assert_eq!(
        def.display(),
        Display::TV(TV::Color),
        "display wins over the legacy monitor key"
    );

    // The detail pane's save path (`HardwareDTO::from_config`) rewrites the hardware section
    // without the legacy key.
    let mut def = def.clone();
    def.hardware = HardwareDTO::from_config(
        &def.to_machine_config().expect("should validate"),
        def.display(),
        None,
    );
    save(dir.path(), "legacy", &def).expect("save should succeed");
    let contents = fs::read_to_string(dir.path().join("legacy.toml")).unwrap();
    assert!(
        !contents.contains("monitor"),
        "a re-save must not write the superseded key:\n{contents}"
    );
    assert!(contents.contains("display = \"tv\""), "{contents}");
}

#[test]
fn future_schema_errors_without_hiding_other_files() {
    let dir = TempDir::new("schema");
    fs::write(
        dir.path().join("too-new.toml"),
        r#"
schema = 2
name = "From the future"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
"#,
    )
    .unwrap();
    let good = full_def();
    save(dir.path(), "good", &good).unwrap();

    let err = load_all(dir.path()).expect_err("a too-new schema must fail the whole load");
    assert!(
        err.contains('2'),
        "error should name the file's schema: {err}"
    );
    assert!(
        err.contains(&CURRENT_SCHEMA.to_string()),
        "error should name the supported schema: {err}"
    );
}

#[test]
fn invalid_hardware_combination_errors_via_validate() {
    let dir = TempDir::new("invalid-hw");
    // CoCo 2 + PAL: rejected by MachineConfig::validate (plain MC6847 PAL timing isn't modeled).
    fs::write(
        dir.path().join("bad.toml"),
        r#"
schema = 1
name = "Bad CoCo 2"

[hardware]
variant = "coco2"
ram = "64k"
video = "pal"
monitor = "rgb"
"#,
    )
    .unwrap();
    let err = load_all(dir.path()).expect_err("PAL CoCo 2 must fail the load");
    assert!(err.contains("PAL"), "error should say why: {err}");
}

#[test]
fn slugify_cases() {
    assert_eq!(slugify("Dev CoCo 3"), "dev-coco-3");
    assert_eq!(slugify("Éric's CoCo 3!!!"), "ric-s-coco-3");
    assert_eq!(slugify("   "), "machine");
    assert_eq!(slugify(""), "machine");
    assert_eq!(slugify("___"), "machine");
    assert_eq!(slugify("already-a-slug"), "already-a-slug");
    assert_eq!(slugify("Multiple   Spaces"), "multiple-spaces");
}

#[test]
fn unique_slug_appends_numeric_suffix() {
    let taken = |s: &str| matches!(s, "dev" | "dev-2" | "dev-3");
    assert_eq!(unique_slug("dev", &taken), "dev-4");
    assert_eq!(unique_slug("fresh", &taken), "fresh");
}

#[test]
fn unknown_keys_still_load() {
    let dir = TempDir::new("unknown-keys");
    fs::write(
        dir.path().join("extra.toml"),
        r#"
schema = 1
name = "Has Extras"
future_top_level_field = true

[drivewire]
enabled = true
hdbdos_mode = false
disk0 = "dw0.dsk"
future_drivewire_field = "preserve me"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = false
future_ui_field = 42
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("unknown keys must warn, not fail");
    assert_eq!(loaded.len(), 1);
}

/// A Save must not erase keys this build doesn't understand — the loader
/// treats them as forward-compatible, and the detail pane round-trips every
/// definition through `save` on every edit.
#[test]
fn save_preserves_unknown_keys() {
    let dir = TempDir::new("preserve-unknown");
    fs::write(
        dir.path().join("extra.toml"),
        r#"
schema = 1
name = "Has Extras"
future_top_level_field = true

[drivewire]
enabled = true
hdbdos_mode = false
disk0 = "dw0.dsk"
future_drivewire_field = "preserve me"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = false
future_ui_field = 42
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("should parse despite unknown keys");
    let mut def = loaded[0].1.clone();

    // A real edit through the detail pane's flow: change something the form owns, then save.
    def.name = "Has Extras (renamed)".to_string();
    save(dir.path(), "extra", &def).expect("save should succeed");

    let contents = fs::read_to_string(dir.path().join("extra.toml")).unwrap();
    let table: toml::Table = toml::from_str(&contents).unwrap();
    assert_eq!(
        table.get("future_top_level_field"),
        Some(&toml::Value::Boolean(true))
    );
    assert_eq!(
        table["hardware"].get("future_hardware_field"),
        Some(&toml::Value::String("whatever".to_string()))
    );
    assert!(
        table["ui"].get("aspect_correct").is_none(),
        "the retired display preference must disappear on save"
    );
    assert_eq!(
        table["ui"].get("future_ui_field"),
        Some(&toml::Value::Integer(42))
    );
    assert_eq!(
        table["drivewire"].get("future_drivewire_field"),
        Some(&toml::Value::String("preserve me".to_string()))
    );

    // And the edit itself did take effect.
    assert_eq!(
        table["name"],
        toml::Value::String("Has Extras (renamed)".to_string())
    );
}

#[test]
fn save_is_atomic_no_leftover_tmp_file() {
    let dir = TempDir::new("atomic-save");
    let def = full_def();
    save(dir.path(), "dev-coco-3", &def).expect("save should succeed");

    let tmp_path = dir.path().join("dev-coco-3.toml.tmp");
    assert!(!tmp_path.exists(), "no .tmp file should remain after save");

    let final_path = dir.path().join("dev-coco-3.toml");
    let contents = fs::read_to_string(&final_path).expect("final file should exist");
    let parsed: MachineDef = toml::from_str(&contents).expect("saved file should parse");
    assert_eq!(parsed, def);
}

/// [`MachineDef::from_config`] must round-trip back through
/// [`MachineDef::to_machine_config`] to exactly the config it was built from.
#[test]
fn from_config_round_trips_through_to_machine_config() {
    let config = MachineConfig {
        variant: MachineVariant::Coco2,
        video: VideoStandard::NTSC,
        memory: MemorySize::K16,
        monitor: None,
        vdg: Some(VDGVariant::MC6847T1),
    };
    let def = MachineDef::from_config("Test CoCo 2".to_string(), None, &config);
    assert_eq!(def.hardware.vdg, Some(VDGVariantDTO::MC6847T1));
    let round_tripped = def
        .to_machine_config()
        .expect("from_config produces a valid def");
    // MachineConfig has no PartialEq derive — compare fields directly.
    assert_eq!(round_tripped.variant, config.variant);
    assert_eq!(round_tripped.video, config.video);
    assert_eq!(round_tripped.memory, config.memory);
    assert_eq!(round_tripped.monitor, config.monitor);
    assert_eq!(round_tripped.vdg, config.vdg);
}

#[test]
fn resolve_media_path_leaves_absolute_paths_untouched() {
    let abs = if cfg!(windows) {
        r"C:\shared\utils.dsk"
    } else {
        "/shared/utils.dsk"
    };
    assert_eq!(resolve_media_path(abs, "dev-coco-3"), PathBuf::from(abs));
}

#[test]
fn resolve_media_path_resolves_relative_against_the_slugs_artifact_dir() {
    let resolved = resolve_media_path("dev.dsk", "dev-coco-3");
    let data_dir = paths::data_dir().expect("home dir should exist in tests");
    assert_eq!(
        resolved,
        data_dir.join("machines").join("dev-coco-3").join("dev.dsk")
    );
}

#[test]
fn missing_dir_returns_empty_list() {
    let dir = std::env::temp_dir().join("coco-egui-machine-def-test-does-not-exist");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        load_all(&dir)
            .expect("a missing dir is the first-run case")
            .is_empty()
    );
}
