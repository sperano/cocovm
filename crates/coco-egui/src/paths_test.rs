use super::*;

#[test]
fn dirs_end_with_app_name() {
    let cfg = config_dir().expect("home dir should exist in tests");
    assert!(cfg.ends_with("cocovm"), "unexpected config dir: {cfg:?}");
    let data = data_dir().expect("home dir should exist in tests");
    assert!(data.ends_with("cocovm"), "unexpected data dir: {data:?}");
}

/// `crates/test-assets/src/lib.rs`'s `xdg_data_dir` duplicates this module's
/// `strategy()`'s `AppStrategyArgs` literal by hand (no dependency edge runs
/// the other way to share it). This test is that duplication's enforcement:
/// if the two literals ever drift, `data_dir()` and `test_assets::xdg_data_dir()`
/// stop agreeing and this fails.
#[test]
fn data_dir_matches_test_assets_xdg_data_dir() {
    assert_eq!(data_dir(), test_assets::xdg_data_dir());
}
