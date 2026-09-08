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
    ])
    .expect("flags parse");
    let file = FileConfig {
        log_level: Some(LogLevel::Error),
        control_port: Some(9999),
        assets_url: Some("https://file.example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(false),
    };
    let config = resolve(cli, file);
    assert_eq!(config.log_level, LogLevel::Trace);
    assert_eq!(config.control_port, 1234);
    assert_eq!(config.assets_url, "https://cli.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
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
fn file_value_beats_built_in_default() {
    if !no_relevant_env_vars_set() {
        return;
    }
    let file = FileConfig {
        log_level: Some(LogLevel::Debug),
        control_port: Some(4242),
        assets_url: Some("https://file.example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(true),
    };
    let config = resolve(bare_cli(), file);
    assert_eq!(config.log_level, LogLevel::Debug);
    assert_eq!(config.control_port, 4242);
    assert_eq!(config.assets_url, "https://file.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
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
    assert_eq!(load(Some(path)), Ok(FileConfig::default()));
}

#[test]
fn malformed_config_file_error_names_the_path() {
    let dir = TempDir::new("config-malformed");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "log_level = [this is not valid toml").unwrap();
    let err = load(Some(path.clone())).expect_err("malformed TOML must fail");
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
        "#,
    )
    .unwrap();
    let file = load(Some(path)).expect("valid config must load");
    assert_eq!(
        file,
        FileConfig {
            log_level: Some(LogLevel::Debug),
            control_port: Some(7000),
            assets_url: Some("https://example.test/bundle.tgz".to_string()),
            toolbar_icons_only: Some(true),
        }
    );
}
