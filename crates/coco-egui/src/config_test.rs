use clap::Parser as _;

use super::*;
use crate::machine_def::tests::TempDir;

/// `Cli::try_parse_from` without any of this module's flags — used to stand
/// in for "neither a flag nor an env var gave this parameter", the same way
/// `cli_test.rs` does. Skipped whenever a relevant `COCOVM_*`/`RUST_LOG`
/// env var is set in the test process, for the same reason those tests
/// guard themselves: clap's `env` fallback would otherwise make this flaky
/// depending on the ambient environment, and mutating process env here would
/// race other tests running in parallel.
fn bare_cli() -> Cli {
    Cli::try_parse_from(["cocovm"]).expect("no required args")
}

fn no_relevant_env_vars_set() -> bool {
    [
        "COCOVM_LOG_LEVEL",
        "COCOVM_CONTROL_PORT",
        "COCOVM_ASSETS_URL",
        "COCOVM_TOOLBAR_ICONS_ONLY",
        "COCOVM_STATUS_BAR_ICONS_ONLY",
    ]
    .iter()
    .all(|var| std::env::var_os(var).is_none())
}

#[test]
fn cli_flag_beats_file_and_default() {
    let cli = Cli::try_parse_from([
        "cocovm",
        "--log-level",
        "trace",
        "--control-port",
        "1234",
        "--assets-url",
        "https://cli.example.test/bundle.tgz",
        "--toolbar-icons-only",
        "--status-bar-icons-only",
    ])
    .expect("flags parse");
    let file = FileConfig {
        log_level: Some(LogLevel::Error),
        control_port: Some(9999),
        assets_url: Some("https://file.example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(false),
        status_bar_icons_only: Some(false),
    };
    let config = resolve(cli, file);
    assert_eq!(config.log_level, LogLevel::Trace);
    assert_eq!(config.control_port, 1234);
    assert_eq!(config.assets_url, "https://cli.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
    assert!(config.toolbar_icons_only_overridden);
    assert!(config.status_bar_icons_only);
    assert!(config.status_bar_icons_only_overridden);
}

#[test]
fn toolbar_icons_only_flag_can_explicitly_override_a_true_file_value_to_false() {
    let cli = Cli::try_parse_from(["cocovm", "--toolbar-icons-only=false"]).expect("flag parses");
    let file = FileConfig {
        toolbar_icons_only: Some(true),
        ..FileConfig::default()
    };
    assert!(!resolve(cli, file).toolbar_icons_only);
}

#[test]
fn status_bar_icons_only_flag_can_explicitly_override_a_true_file_value_to_false() {
    let cli =
        Cli::try_parse_from(["cocovm", "--status-bar-icons-only=false"]).expect("flag parses");
    let file = FileConfig {
        status_bar_icons_only: Some(true),
        ..FileConfig::default()
    };
    assert!(!resolve(cli, file).status_bar_icons_only);
}

#[test]
fn file_value_beats_built_in_default() {
    if !no_relevant_env_vars_set() {
        return;
    }
    let file = FileConfig {
        log_level: Some(LogLevel::Debug),
        control_port: Some(4242),
        assets_url: Some("https://file.example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(true),
        status_bar_icons_only: Some(true),
    };
    let config = resolve(bare_cli(), file);
    assert_eq!(config.log_level, LogLevel::Debug);
    assert_eq!(config.control_port, 4242);
    assert_eq!(config.assets_url, "https://file.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
    assert!(!config.toolbar_icons_only_overridden);
    assert!(config.status_bar_icons_only);
    assert!(!config.status_bar_icons_only_overridden);
}

#[test]
fn built_in_defaults_apply_when_nothing_else_is_set() {
    if !no_relevant_env_vars_set() {
        return;
    }
    let config = resolve(bare_cli(), FileConfig::default());
    assert_eq!(config.log_level, LogLevel::Warn);
    assert_eq!(config.control_port, crate::control::DEFAULT_PORT);
    assert_eq!(config.assets_url, crate::startup::DEFAULT_ASSETS_URL);
    assert!(!config.toolbar_icons_only);
    assert!(!config.status_bar_icons_only);
}

#[test]
fn missing_config_dir_yields_the_empty_file_config() {
    assert_eq!(load(None), Ok(FileConfig::default()));
}

#[test]
fn missing_config_file_yields_the_empty_file_config() {
    let dir = TempDir::new("config-missing-file");
    let path = dir.path().join("config.toml");
    assert!(!path.is_file());
    assert_eq!(load(Some(&path)), Ok(FileConfig::default()));
}

#[test]
fn malformed_config_file_error_names_the_path() {
    let dir = TempDir::new("config-malformed");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "log_level = [this is not valid toml").unwrap();
    let err = load(Some(&path)).expect_err("malformed TOML must fail");
    assert!(
        err.contains(&path.display().to_string()),
        "error must name the file: {err}"
    );
}

#[test]
fn unknown_key_is_rejected() {
    let err = toml::from_str::<FileConfig>("nonexistent_setting = true")
        .expect_err("deny_unknown_fields must reject an unrecognized key");
    assert!(err.to_string().contains("nonexistent_setting"));
}

#[test]
fn log_level_strings_match_the_cli_flags_spelling() {
    let pairs = [
        ("error", LogLevel::Error),
        ("warn", LogLevel::Warn),
        ("info", LogLevel::Info),
        ("debug", LogLevel::Debug),
        ("trace", LogLevel::Trace),
    ];
    for (text, level) in pairs {
        let toml = format!("log_level = \"{text}\"");
        let parsed: FileConfig = toml::from_str(&toml).unwrap_or_else(|e| panic!("{toml}: {e}"));
        assert_eq!(parsed.log_level, Some(level), "{text}");
    }
}

/// Uncommented copy of [`default_config_template`]'s parameter lines, so it
/// can be parsed back as a [`FileConfig`]. Only lines that are unambiguously
/// a commented parameter assignment (`# name = ...` with a lowercase/
/// underscore `name` up to the first `" = "`) get uncommented; prose header
/// lines and description lines like `# error | warn | ...` are left alone —
/// as long as no such prose line happens to contain `" = "` after a
/// lowercase/underscore run, which would make it spuriously uncommented.
/// That failure mode isn't silent: `FileConfig`'s `deny_unknown_fields`
/// rejects any resulting key that isn't one of the five real parameters.
fn uncomment_template_parameters(template: &str) -> String {
    template
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("# ") else {
                return line.to_string();
            };
            let Some(eq_pos) = rest.find(" = ") else {
                return line.to_string();
            };
            let candidate = &rest[..eq_pos];
            let is_identifier = !candidate.is_empty()
                && candidate
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_');
            if is_identifier {
                rest.to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn default_template_uncommented_resolves_to_true_defaults() {
    if !no_relevant_env_vars_set() {
        return;
    }
    let uncommented = uncomment_template_parameters(&default_config_template());
    let file: FileConfig =
        toml::from_str(&uncommented).unwrap_or_else(|e| panic!("{uncommented}: {e}"));

    // Destructured without `..`: a field added to `FileConfig` later must be
    // named here too, or this line fails to compile — the drift guard below
    // only pins the values of parameters the template already covers, so
    // this is what pins *coverage* (every field gets a commented line).
    let FileConfig {
        log_level,
        control_port,
        assets_url,
        toolbar_icons_only,
        status_bar_icons_only,
    } = &file;
    assert!(
        log_level.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        control_port.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        assets_url.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        toolbar_icons_only.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        status_bar_icons_only.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );

    let config = resolve(bare_cli(), file);
    let expected = resolve(bare_cli(), FileConfig::default());
    assert_eq!(config, expected);
}

#[test]
fn seed_default_file_creates_file_with_the_template_content() {
    let dir = TempDir::new("config-seed-new");
    let path = dir.path().join("config.toml");
    seed_default_file(&path);
    let contents = std::fs::read_to_string(&path).expect("file must be created");
    assert_eq!(contents, default_config_template());
}

#[test]
fn seed_default_file_is_a_no_op_when_the_file_already_exists() {
    let dir = TempDir::new("config-seed-existing");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "log_level = \"debug\"\n").unwrap();
    seed_default_file(&path);
    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(contents, "log_level = \"debug\"\n");
}

#[test]
fn seed_default_file_creates_missing_parent_directories() {
    let dir = TempDir::new("config-seed-nested");
    let path = dir.path().join("nested").join("config.toml");
    assert!(!path.parent().unwrap().is_dir());
    seed_default_file(&path);
    let contents = std::fs::read_to_string(&path).expect("file must be created");
    assert_eq!(contents, default_config_template());
}

#[test]
fn freshly_seeded_file_loads_as_the_empty_file_config() {
    let dir = TempDir::new("config-seed-loads");
    let path = dir.path().join("config.toml");
    seed_default_file(&path);
    assert_eq!(load(Some(&path)), Ok(FileConfig::default()));
}

#[test]
fn a_valid_full_config_file_loads() {
    let dir = TempDir::new("config-full");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
        log_level = "debug"
        control_port = 7000
        assets_url = "https://example.test/bundle.tgz"
        toolbar_icons_only = true
        status_bar_icons_only = true
        "#,
    )
    .unwrap();
    let file = load(Some(&path)).expect("valid config must load");
    assert_eq!(
        file,
        FileConfig {
            log_level: Some(LogLevel::Debug),
            control_port: Some(7000),
            assets_url: Some("https://example.test/bundle.tgz".to_string()),
            toolbar_icons_only: Some(true),
            status_bar_icons_only: Some(true),
        }
    );
}

#[test]
fn save_file_round_trips_through_load() {
    let dir = TempDir::new("config-save-roundtrip");
    let path = dir.path().join("config.toml");
    let file = FileConfig {
        log_level: Some(LogLevel::Debug),
        control_port: Some(7001),
        assets_url: Some("https://example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(true),
        status_bar_icons_only: Some(true),
    };
    save_file(&path, &file).expect("save must succeed");
    assert_eq!(load(Some(&path)).expect("saved file must load"), file);
}

#[test]
fn save_file_preserves_template_comments_while_updating_a_key() {
    let dir = TempDir::new("config-save-preserves-comments");
    let path = dir.path().join("config.toml");
    seed_default_file(&path);
    let template = std::fs::read_to_string(&path).unwrap();

    save_file(
        &path,
        &FileConfig {
            control_port: Some(7002),
            ..FileConfig::default()
        },
    )
    .expect("save must succeed");

    let saved = std::fs::read_to_string(&path).unwrap();
    for line in template.lines().filter(|l| l.trim_start().starts_with('#')) {
        assert!(saved.contains(line), "comment line lost: {line}\n{saved}");
    }
    assert!(saved.contains("control_port = 7002"), "{saved}");
}

#[test]
fn save_file_removes_a_key_set_back_to_none() {
    let dir = TempDir::new("config-save-remove-key");
    let path = dir.path().join("config.toml");
    save_file(
        &path,
        &FileConfig {
            control_port: Some(7003),
            ..FileConfig::default()
        },
    )
    .expect("first save must succeed");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .any(|l| l.trim_start().starts_with("control_port"))
    );

    save_file(&path, &FileConfig::default()).expect("second save must succeed");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        !saved
            .lines()
            .any(|l| l.trim_start().starts_with("control_port")),
        "control_port key must be removed: {saved}"
    );
}

#[test]
fn save_file_with_no_existing_file_starts_from_the_template() {
    let dir = TempDir::new("config-save-no-file");
    let path = dir.path().join("config.toml");
    assert!(!path.is_file());

    save_file(
        &path,
        &FileConfig {
            log_level: Some(LogLevel::Trace),
            ..FileConfig::default()
        },
    )
    .expect("save must succeed");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("# cocovm global config."),
        "template header must survive: {saved}"
    );
    assert!(saved.contains("log_level = \"trace\""), "{saved}");
    assert!(
        !saved
            .lines()
            .any(|l| l.trim_start().starts_with("control_port")),
        "untouched field must stay unset: {saved}"
    );
}
