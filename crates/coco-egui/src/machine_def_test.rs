use super::*;
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
            ram: RamDTO::K512,
            video: VideoStandardDTO::NTSC,
            monitor: Some(MonitorDTO::RGB),
            vdg: None,
            rom: Some("/path/custom.rom".to_string()),
        },
        media: MediaDTO {
            cart: Some("/paks/arkanoid.ccc".to_string()),
            disk0: Some("dev.dsk".to_string()),
            disk1: Some("/shared/utils.dsk".to_string()),
            vhd0: Some("/vhd/68SDC.VHD".to_string()),
            vhd1: None,
            tape: Some("session.cas".to_string()),
        },
        peripherals: PeripheralsDTO {
            mpi: true,
            rtc: true,
            fd502: true,
        },
        ui: UIDTO {
            aspect_correct: false,
            kb_mode: KbModeDTO::Symbolic,
        },
        unknown: toml::Table::new(),
    }
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
    assert!(!def.peripherals.mpi);
    assert!(!def.peripherals.rtc);
    assert!(def.ui.aspect_correct);
    assert_eq!(def.ui.kb_mode, KbModeDTO::Positional);

    // Default VDG is per-variant: a CoCo 3 has none at all.
    let config = def.to_machine_config().expect("should validate");
    assert_eq!(config.vdg, None);
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

    let err = load_all(dir.path())
        .expect_err("a too-new schema must fail the whole load");
    assert!(err.contains('2'), "error should name the file's schema: {err}");
    assert!(
        err.contains(&CURRENT_SCHEMA.to_string()),
        "error should name the supported schema: {err}"
    );
}

#[test]
fn invalid_hardware_combination_errors_via_validate() {
    let dir = TempDir::new("invalid-hw");
    // CoCo 2 + PAL: rejected by MachineConfig::validate (plain MC6847
    // PAL timing isn't modeled).
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

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = true
future_ui_field = 42
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("unknown keys must warn, not fail");
    assert_eq!(loaded.len(), 1);
}

/// A Save must not erase keys this build doesn't understand — the
/// loader treats them as forward-compatible (`unknown_keys_still_load`
/// above), and the manager's detail pane round-trips every loaded
/// definition through `save` on every edit, so losing them there would
/// contradict that compatibility story.
#[test]
fn save_preserves_unknown_keys() {
    let dir = TempDir::new("preserve-unknown");
    fs::write(
        dir.path().join("extra.toml"),
        r#"
schema = 1
name = "Has Extras"
future_top_level_field = true

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"
future_hardware_field = "whatever"

[ui]
aspect_correct = true
future_ui_field = 42
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("should parse despite unknown keys");
    let mut def = loaded[0].1.clone();

    // A real edit through the detail pane's flow: change something the
    // form actually owns, then save.
    def.name = "Has Extras (renamed)".to_string();
    save(dir.path(), "extra", &def).expect("save should succeed");

    let contents = fs::read_to_string(dir.path().join("extra.toml")).unwrap();
    let table: toml::Table = toml::from_str(&contents).unwrap();
    assert_eq!(table.get("future_top_level_field"), Some(&toml::Value::Boolean(true)));
    assert_eq!(
        table["hardware"].get("future_hardware_field"),
        Some(&toml::Value::String("whatever".to_string()))
    );
    assert_eq!(table["ui"].get("future_ui_field"), Some(&toml::Value::Integer(42)));

    // And the edit itself did take effect.
    assert_eq!(table["name"], toml::Value::String("Has Extras (renamed)".to_string()));
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

/// [`MachineDef::from_config`] (the manager's "New…" Create path) must
/// round-trip back through [`MachineDef::to_machine_config`] to exactly
/// the config it was built from.
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
    let round_tripped = def.to_machine_config().expect("from_config produces a valid def");
    // MachineConfig has no PartialEq derive — compare fields directly.
    assert_eq!(round_tripped.variant, config.variant);
    assert_eq!(round_tripped.video, config.video);
    assert_eq!(round_tripped.memory, config.memory);
    assert_eq!(round_tripped.monitor, config.monitor);
    assert_eq!(round_tripped.vdg, config.vdg);
}

#[test]
fn resolve_media_path_leaves_absolute_paths_untouched() {
    let abs = if cfg!(windows) { r"C:\shared\utils.dsk" } else { "/shared/utils.dsk" };
    assert_eq!(resolve_media_path(abs, "dev-coco-3"), PathBuf::from(abs));
}

#[test]
fn resolve_media_path_resolves_relative_against_the_slugs_artifact_dir() {
    let resolved = resolve_media_path("dev.dsk", "dev-coco-3");
    let data_dir = paths::data_dir().expect("home dir should exist in tests");
    assert_eq!(resolved, data_dir.join("machines").join("dev-coco-3").join("dev.dsk"));
}

#[test]
fn missing_dir_returns_empty_list() {
    let dir = std::env::temp_dir().join("coco-egui-machine-def-test-does-not-exist");
    let _ = fs::remove_dir_all(&dir);
    assert!(load_all(&dir).expect("a missing dir is the first-run case").is_empty());
}
