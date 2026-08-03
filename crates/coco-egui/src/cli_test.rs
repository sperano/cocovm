use super::*;

#[test]
fn parse_machine_accepts_known_spellings_and_rejects_others() {
    assert_eq!(parse_machine("coco1"), Ok(MachineVariant::Coco1));
    assert_eq!(parse_machine("coco2"), Ok(MachineVariant::Coco2));
    assert_eq!(parse_machine("coco3"), Ok(MachineVariant::Coco3));
    assert!(parse_machine("coco4").is_err());
    assert!(parse_machine("").is_err());
}

#[test]
fn parse_ram_accepts_every_memory_size_spelling() {
    assert_eq!(parse_ram("4k"), Ok(MemorySize::K4));
    assert_eq!(parse_ram("16k"), Ok(MemorySize::K16));
    assert_eq!(parse_ram("32k"), Ok(MemorySize::K32));
    assert_eq!(parse_ram("64k"), Ok(MemorySize::K64));
    assert_eq!(parse_ram("128k"), Ok(MemorySize::K128));
    assert_eq!(parse_ram("512k"), Ok(MemorySize::K512));
    assert_eq!(parse_ram("2048k"), Ok(MemorySize::K2048));
    assert!(parse_ram("1mb").is_err());
}

#[test]
fn parse_video_accepts_ntsc_and_pal() {
    assert_eq!(parse_video("ntsc"), Ok(VideoStandard::NTSC));
    assert_eq!(parse_video("pal"), Ok(VideoStandard::PAL));
    assert!(parse_video("secam").is_err());
}

#[test]
fn default_ram_is_512k_for_coco3_and_64k_for_coco1_2() {
    assert_eq!(default_ram(MachineVariant::Coco3), MemorySize::K512);
    assert_eq!(default_ram(MachineVariant::Coco1), MemorySize::K64);
    assert_eq!(default_ram(MachineVariant::Coco2), MemorySize::K64);
}

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

#[test]
fn default_vdg_is_t1_for_coco2_and_plain_elsewhere() {
    assert_eq!(
        default_vdg(MachineVariant::Coco2),
        Some(VDGVariant::MC6847T1)
    );
    assert_eq!(default_vdg(MachineVariant::Coco1), Some(VDGVariant::MC6847));
    assert_eq!(default_vdg(MachineVariant::Coco3), None);
}
