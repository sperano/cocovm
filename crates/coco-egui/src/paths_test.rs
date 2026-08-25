use super::*;

#[test]
fn dirs_end_with_app_name() {
    let cfg = config_dir().expect("home dir should exist in tests");
    assert!(cfg.ends_with("cocovm"), "unexpected config dir: {cfg:?}");
    let data = data_dir().expect("home dir should exist in tests");
    assert!(data.ends_with("cocovm"), "unexpected data dir: {data:?}");
}

/// Guards against `test-assets`' hand-duplicated `AppStrategyArgs` literal
/// drifting from this module's `strategy()`.
#[test]
fn data_dir_matches_test_assets_xdg_data_dir() {
    assert_eq!(data_dir(), test_assets::xdg_data_dir());
}
