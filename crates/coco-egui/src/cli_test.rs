use super::*;

#[test]
fn log_level_maps_onto_its_level_filter() {
    let pairs = [
        (LogLevel::Error, LevelFilter::ERROR),
        (LogLevel::Warn, LevelFilter::WARN),
        (LogLevel::Info, LevelFilter::INFO),
        (LogLevel::Debug, LevelFilter::DEBUG),
        (LogLevel::Trace, LevelFilter::TRACE),
    ];
    for (level, filter) in pairs {
        assert_eq!(LevelFilter::from(level), filter, "{level:?}");
    }
}

#[test]
fn log_level_comes_from_the_flag() {
    use clap::Parser as _;

    // try_parse_from avoids exiting the test binary on a bad arg; .ok() since clap::Error isn't
    // PartialEq.
    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.log_level).ok();
    assert_eq!(
        parse(&["cocovm", "--log-level", "debug"]),
        Some(Some(LogLevel::Debug))
    );
    assert_eq!(
        parse(&["cocovm", "-L", "trace"]),
        Some(Some(LogLevel::Trace))
    );
    assert_eq!(parse(&["cocovm", "-L", "chatty"]), None);
    // Skipped when COCOVM_LOG_LEVEL is set: it outranks the flag's absence, and can't be cleared
    // here without racing other tests.
    if std::env::var_os("COCOVM_LOG_LEVEL").is_none() {
        assert_eq!(parse(&["cocovm"]), Some(None));
    }
}

#[test]
fn machine_slug_comes_from_the_positional_parameter() {
    use clap::Parser as _;

    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.machine).ok();
    assert_eq!(parse(&["cocovm"]), Some(None));
    assert_eq!(
        parse(&["cocovm", "my-coco"]),
        Some(Some("my-coco".to_string()))
    );
    assert_eq!(
        parse(&["cocovm", "--log-level", "info", "my-coco"]),
        Some(Some("my-coco".to_string()))
    );
}

#[test]
fn assets_url_comes_from_the_flag() {
    use clap::Parser as _;

    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.assets_url).ok();
    assert_eq!(
        parse(&["cocovm", "--assets-url", "https://example.test/bundle.tgz"]),
        Some(Some("https://example.test/bundle.tgz".to_string()))
    );
    // Skipped when COCOVM_ASSETS_URL is set, same reason as the log-level test.
    if std::env::var_os("COCOVM_ASSETS_URL").is_none() {
        assert_eq!(parse(&["cocovm"]), Some(None));
    }
}

/// The default with no flag, no env, and no config file — `resolve()`'s
/// job, not the bare `Cli` parse (`config_test.rs` covers the rest of the
/// precedence chain). Skipped under the same ambient-env-var conditions as
/// the tests above.
#[test]
fn log_level_and_assets_url_default_through_resolve() {
    use clap::Parser as _;

    if std::env::var_os("COCOVM_LOG_LEVEL").is_some()
        || std::env::var_os("COCOVM_ASSETS_URL").is_some()
    {
        return;
    }
    let cli = Cli::try_parse_from(["cocovm"]).expect("no required args");
    let config = crate::config::resolve(cli, crate::config::FileConfig::default());
    assert_eq!(config.log_level, LogLevel::Warn);
    assert_eq!(config.assets_url, crate::startup::DEFAULT_ASSETS_URL);
}

#[test]
fn toolbar_icons_only_bare_flag_means_true_and_takes_an_explicit_value() {
    use clap::Parser as _;

    let parse = |args: &[&str]| {
        Cli::try_parse_from(args)
            .map(|cli| cli.toolbar_icons_only)
            .ok()
    };
    assert_eq!(parse(&["cocovm"]), Some(None));
    assert_eq!(parse(&["cocovm", "--toolbar-icons-only"]), Some(Some(true)));
    assert_eq!(
        parse(&["cocovm", "--toolbar-icons-only=false"]),
        Some(Some(false))
    );
}

#[test]
fn status_bar_icons_only_bare_flag_means_true_and_takes_an_explicit_value() {
    use clap::Parser as _;

    let parse = |args: &[&str]| {
        Cli::try_parse_from(args)
            .map(|cli| cli.status_bar_icons_only)
            .ok()
    };
    assert_eq!(parse(&["cocovm"]), Some(None));
    assert_eq!(
        parse(&["cocovm", "--status-bar-icons-only"]),
        Some(Some(true))
    );
    assert_eq!(
        parse(&["cocovm", "--status-bar-icons-only=false"]),
        Some(Some(false))
    );
}

#[test]
fn welcome_image_flags_parse_and_reject_a_zero_interval() {
    use clap::Parser as _;

    let parse = |args: &[&str]| {
        Cli::try_parse_from(args)
            .map(|cli| {
                (
                    cli.welcome_image_cycle,
                    cli.welcome_image_cycle_secs,
                    cli.welcome_image_shuffle,
                )
            })
            .ok()
    };
    assert_eq!(parse(&["cocovm"]), Some((None, None, None)));
    assert_eq!(
        parse(&["cocovm", "--welcome-image-cycle"]),
        Some((Some(true), None, None))
    );
    assert_eq!(
        parse(&["cocovm", "--welcome-image-cycle=false"]),
        Some((Some(false), None, None))
    );
    assert_eq!(
        parse(&["cocovm", "--welcome-image-cycle-secs", "15"]),
        Some((None, std::num::NonZeroU32::new(15), None))
    );
    assert_eq!(parse(&["cocovm", "--welcome-image-cycle-secs", "0"]), None);
    assert_eq!(
        parse(&["cocovm", "--welcome-image-shuffle"]),
        Some((None, None, Some(true)))
    );
    assert_eq!(
        parse(&["cocovm", "--welcome-image-shuffle=no"]),
        Some((None, None, Some(false)))
    );
}

/// `BoolishValueParser` — the dominant env-var boolean spelling
/// (`COCOVM_TOOLBAR_ICONS_ONLY=1`), not just `true`/`false`.
#[test]
fn toolbar_icons_only_accepts_boolish_spellings() {
    use clap::Parser as _;

    let parse = |value: &str| {
        Cli::try_parse_from(["cocovm", &format!("--toolbar-icons-only={value}")])
            .map(|cli| cli.toolbar_icons_only)
            .ok()
    };
    for value in ["1", "yes", "y", "on"] {
        assert_eq!(parse(value), Some(Some(true)), "{value}");
    }
    for value in ["0", "no", "n", "off"] {
        assert_eq!(parse(value), Some(Some(false)), "{value}");
    }
}

#[test]
fn check_for_updates_bare_flag_means_true_and_takes_an_explicit_value() {
    use clap::Parser as _;

    let parse = |args: &[&str]| {
        Cli::try_parse_from(args)
            .map(|cli| cli.check_for_updates)
            .ok()
    };
    // Skipped when COCOVM_CHECK_FOR_UPDATES is set, like `log_level_comes_from_the_flag`.
    if std::env::var_os("COCOVM_CHECK_FOR_UPDATES").is_none() {
        assert_eq!(parse(&["cocovm"]), Some(None));
    }
    assert_eq!(parse(&["cocovm", "--check-for-updates"]), Some(Some(true)));
    assert_eq!(
        parse(&["cocovm", "--check-for-updates=off"]),
        Some(Some(false))
    );
}
