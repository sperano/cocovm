use clap::Parser as _;

use super::*;
use crate::machine_def::tests::TempDir;

fn hotkey(text: &str) -> Option<Hotkey> {
    Some(text.parse().expect("valid hotkey"))
}

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
        "COCOVM_WELCOME_IMAGE_CYCLE",
        "COCOVM_WELCOME_IMAGE_CYCLE_SECS",
        "COCOVM_WELCOME_IMAGE_SHUFFLE",
        "COCOVM_CHECK_FOR_UPDATES",
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
        "--welcome-image-cycle",
        "--welcome-image-cycle-secs",
        "5",
        "--welcome-image-shuffle",
        "--check-for-updates=false",
    ])
    .expect("flags parse");
    let file = FileConfig {
        log_level: Some(LogLevel::Error),
        control_port: Some(9999),
        assets_url: Some("https://file.example.test/bundle.tgz".to_string()),
        toolbar_icons_only: Some(false),
        status_bar_icons_only: Some(false),
        welcome_image_cycle: Some(false),
        welcome_image_cycle_secs: NonZeroU32::new(99),
        welcome_image_shuffle: Some(false),
        check_for_updates: Some(true),
        manager_sort: Some(ManagerSort::NameDesc),
        ..FileConfig::default()
    };
    let config = resolve(cli, file);
    assert_eq!(config.log_level, LogLevel::Trace);
    assert!(config.log_level_overridden);
    assert_eq!(config.control_port, 1234);
    assert!(config.control_port_overridden);
    assert_eq!(config.assets_url, "https://cli.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
    assert!(config.toolbar_icons_only_overridden);
    assert!(config.status_bar_icons_only);
    assert!(config.status_bar_icons_only_overridden);
    assert!(config.welcome_image_cycle);
    assert!(config.welcome_image_cycle_overridden);
    assert_eq!(config.welcome_image_cycle_secs.get(), 5);
    assert!(config.welcome_image_cycle_secs_overridden);
    assert!(config.welcome_image_shuffle);
    assert!(config.welcome_image_shuffle_overridden);
    assert!(!config.check_for_updates);
    assert_eq!(config.manager_sort, ManagerSort::NameDesc);
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
        welcome_image_cycle: Some(true),
        welcome_image_cycle_secs: NonZeroU32::new(8),
        welcome_image_shuffle: Some(true),
        check_for_updates: Some(false),
        hotkey_key_layout: hotkey("F9"),
        hotkey_keyboard_mode: hotkey("Shift+F9"),
        hotkey_new_machine: hotkey("Cmd+Alt+N"),
        hotkey_debugger: hotkey("Alt+F12"),
        hotkey_load_state_4: hotkey("Cmd+Alt+4"),
        hotkey_save_state_5: hotkey("Cmd+Alt+Shift+5"),
        manager_sort: Some(ManagerSort::NameAsc),
        ..FileConfig::default()
    };
    let config = resolve(bare_cli(), file);
    assert_eq!(config.log_level, LogLevel::Debug);
    assert!(!config.log_level_overridden);
    assert_eq!(config.control_port, 4242);
    assert!(!config.control_port_overridden);
    assert_eq!(config.assets_url, "https://file.example.test/bundle.tgz");
    assert!(config.toolbar_icons_only);
    assert!(!config.toolbar_icons_only_overridden);
    assert!(config.status_bar_icons_only);
    assert!(!config.status_bar_icons_only_overridden);
    assert!(config.welcome_image_cycle);
    assert!(!config.welcome_image_cycle_overridden);
    assert_eq!(config.welcome_image_cycle_secs.get(), 8);
    assert!(!config.welcome_image_cycle_secs_overridden);
    assert!(config.welcome_image_shuffle);
    assert!(!config.welcome_image_shuffle_overridden);
    assert!(!config.check_for_updates);
    assert_eq!(Some(config.hotkeys.key_layout), hotkey("F9"));
    assert_eq!(Some(config.hotkeys.keyboard_mode), hotkey("Shift+F9"));
    assert_eq!(Some(config.hotkeys.new_machine), hotkey("Cmd+Alt+N"));
    assert_eq!(Some(config.hotkeys.debugger), hotkey("Alt+F12"));
    assert_eq!(Some(config.hotkeys.load_state[3]), hotkey("Cmd+Alt+4"));
    assert_eq!(
        Some(config.hotkeys.save_state[4]),
        hotkey("Cmd+Alt+Shift+5")
    );
    assert_eq!(config.hotkeys.load_state[0], DEFAULT_HOTKEYS.load_state[0]);
    assert_eq!(config.manager_sort, ManagerSort::NameAsc);
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
    assert!(config.check_for_updates);
    assert_eq!(config.hotkeys, DEFAULT_HOTKEYS);
    assert_eq!(config.manager_sort, ManagerSort::CreatedDesc);
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
/// digit/underscore `name` up to the first `" = "`) get uncommented; prose
/// header lines and description lines like `# error | warn | ...` are left
/// alone — as long as no such prose line happens to contain `" = "` after a
/// lowercase/digit/underscore run, which would make it spuriously uncommented.
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
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
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
        welcome_image_cycle,
        welcome_image_cycle_secs,
        welcome_image_shuffle,
        check_for_updates,
        hotkey_key_layout: _,
        hotkey_keyboard_mode: _,
        hotkey_new_machine: _,
        hotkey_debugger: _,
        hotkey_load_state_1: _,
        hotkey_load_state_2: _,
        hotkey_load_state_3: _,
        hotkey_load_state_4: _,
        hotkey_load_state_5: _,
        hotkey_save_state_1: _,
        hotkey_save_state_2: _,
        hotkey_save_state_3: _,
        hotkey_save_state_4: _,
        hotkey_save_state_5: _,
        manager_sort,
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
    assert!(
        welcome_image_cycle.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        welcome_image_cycle_secs.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        welcome_image_shuffle.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    assert!(
        check_for_updates.is_some(),
        "every FileConfig parameter needs a commented line in the template"
    );
    // The `hotkey_*` fields are covered through their actions: `hotkey`
    // matches every `HotkeyAction` to its field, so this loop reaches each.
    for action in HotkeyAction::all() {
        assert!(
            file.hotkey(action).is_some(),
            "every FileConfig parameter needs a commented line in the template: {action:?}"
        );
    }
    assert!(
        manager_sort.is_some(),
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
        welcome_image_cycle = true
        welcome_image_cycle_secs = 45
        welcome_image_shuffle = true
        check_for_updates = false
        hotkey_key_layout = "F9"
        hotkey_keyboard_mode = "Shift+F11"
        hotkey_new_machine = "Cmd+Shift+N"
        hotkey_debugger = "Alt+F5"
        hotkey_load_state_5 = "Cmd+Alt+5"
        hotkey_save_state_1 = "Cmd+Alt+Shift+1"
        manager_sort = "name-desc"
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
            welcome_image_cycle: Some(true),
            welcome_image_cycle_secs: NonZeroU32::new(45),
            welcome_image_shuffle: Some(true),
            check_for_updates: Some(false),
            hotkey_key_layout: hotkey("F9"),
            hotkey_keyboard_mode: hotkey("Shift+F11"),
            hotkey_new_machine: hotkey("Cmd+Shift+N"),
            hotkey_debugger: hotkey("Alt+F5"),
            hotkey_load_state_5: hotkey("Cmd+Alt+5"),
            hotkey_save_state_1: hotkey("Cmd+Alt+Shift+1"),
            manager_sort: Some(ManagerSort::NameDesc),
            ..FileConfig::default()
        }
    );
}

/// `set_hotkey` and `hotkey` reach the same field for every action.
#[test]
fn set_hotkey_is_read_back_by_hotkey_for_every_action() {
    let mut file = FileConfig::default();
    let bound = hotkey("Cmd+Alt+F5");
    for action in HotkeyAction::all() {
        file.set_hotkey(action, bound);
        assert_eq!(file.hotkey(action), bound, "{action:?}");
        for other in HotkeyAction::all().filter(|other| *other != action) {
            assert_eq!(file.hotkey(other), None, "{action:?} wrote {other:?}");
        }
        file.set_hotkey(action, None);
    }
    assert_eq!(file, FileConfig::default());
}

/// A state chord is a hotkey like any other: a file giving its default to
/// another action is refused at load, naming both.
#[test]
fn a_hotkey_clashing_with_a_state_chord_is_a_load_error() {
    let dir = TempDir::new("config-state-chord-clash");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "hotkey_new_machine = \"Cmd+4\"\n").unwrap();
    let error = load(Some(&path)).expect_err("Cmd+4 is already Load State 4");
    assert!(error.contains("New machine"), "{error}");
    assert!(error.contains("Load State 4"), "{error}");
}

#[test]
fn an_invalid_hotkey_is_a_load_error_naming_the_key() {
    let dir = TempDir::new("config-bad-hotkey");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "hotkey_key_layout = \"K\"\n").unwrap();
    let error = load(Some(&path)).expect_err("a bare letter must be rejected");
    assert!(error.contains("hotkey_key_layout"), "{error}");
    assert!(error.contains("would type into the machine"), "{error}");
}

/// Each key is valid alone; the clash only exists against the other
/// action's built-in default.
#[test]
fn a_hotkey_clashing_with_another_actions_default_is_a_load_error() {
    let dir = TempDir::new("config-hotkey-clash");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "hotkey_key_layout = \"F12\"\n").unwrap();
    let error = load(Some(&path)).expect_err("F12 is already Keyboard mode");
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains("both F12"), "{error}");
}

#[test]
fn a_zero_welcome_image_cycle_secs_is_a_load_error() {
    let dir = TempDir::new("config-zero-shuffle-secs");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "welcome_image_cycle_secs = 0\n").unwrap();
    let error = load(Some(&path)).expect_err("zero must be rejected");
    assert!(
        error.contains("welcome_image_cycle_secs"),
        "error must name the key: {error}"
    );
}

#[test]
fn an_invalid_manager_sort_is_a_load_error() {
    let dir = TempDir::new("config-invalid-manager-sort");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "manager_sort = \"recent-ish\"\n").unwrap();

    let error = load(Some(&path)).expect_err("unknown sort value must be rejected");

    assert!(
        error.contains("manager_sort"),
        "error must name the key: {error}"
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
        welcome_image_cycle: Some(true),
        welcome_image_cycle_secs: NonZeroU32::new(12),
        welcome_image_shuffle: Some(true),
        check_for_updates: Some(false),
        hotkey_key_layout: hotkey("F9"),
        hotkey_keyboard_mode: hotkey("Alt+F12"),
        hotkey_new_machine: hotkey("Cmd+Shift+N"),
        hotkey_debugger: hotkey("F11"),
        hotkey_load_state_3: hotkey("Cmd+Alt+3"),
        hotkey_save_state_3: hotkey("Cmd+Alt+Shift+3"),
        manager_sort: Some(ManagerSort::CreatedAsc),
        ..FileConfig::default()
    };
    save_file(&path, &file).expect("save must succeed");
    assert_eq!(load(Some(&path)).expect("saved file must load"), file);
}

#[test]
fn save_manager_sort_preserves_other_settings() {
    let dir = TempDir::new("config-save-manager-sort");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# retained comment\ncontrol_port = 7004\nmanager_sort = \"created-desc\"\n",
    )
    .unwrap();

    save_manager_sort(&path, ManagerSort::NameAsc).expect("sort preference saves");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("# retained comment"));
    assert!(saved.contains("control_port = 7004"));
    assert!(saved.contains("manager_sort = \"name-asc\""));
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
