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

    // `try_parse_from` so a bad argument fails the test instead of exiting
    // the test binary; argv[0] stands in for the program name. `ok` because
    // `clap::Error` is not `PartialEq`.
    let parse = |args: &[&str]| Cli::try_parse_from(args).map(|cli| cli.log_level).ok();
    assert_eq!(
        parse(&["cocovm", "--log-level", "debug"]),
        Some(LogLevel::Debug)
    );
    assert_eq!(parse(&["cocovm", "-L", "trace"]), Some(LogLevel::Trace));
    assert_eq!(parse(&["cocovm", "-L", "chatty"]), None);
    // The bare default is only `warn` when the environment isn't speaking:
    // `COCOVM_LOG_LEVEL` outranks it, and env vars can't be cleared here
    // without racing the other tests in this process.
    if std::env::var_os("COCOVM_LOG_LEVEL").is_none() {
        assert_eq!(parse(&["cocovm"]), Some(LogLevel::Warn));
    }
}
