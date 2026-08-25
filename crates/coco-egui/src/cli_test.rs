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
fn log_level_comes_from_the_flag_and_defaults_to_warn() {
    use clap::Parser as _;

    // try_parse_from avoids exiting the test binary on a bad arg; .ok() since clap::Error isn't PartialEq.
    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.log_level).ok();
    assert_eq!(
        parse(&["cocovm", "--log-level", "debug"]),
        Some(LogLevel::Debug)
    );
    assert_eq!(parse(&["cocovm", "-L", "trace"]), Some(LogLevel::Trace));
    assert_eq!(parse(&["cocovm", "-L", "chatty"]), None);
    // Skipped when COCOVM_LOG_LEVEL is set: it outranks the default, and can't be cleared here without racing other tests.
    if std::env::var_os("COCOVM_LOG_LEVEL").is_none() {
        assert_eq!(parse(&["cocovm"]), Some(LogLevel::Warn));
    }
}
