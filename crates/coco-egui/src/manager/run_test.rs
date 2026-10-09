use crate::APP_ID;

const DESKTOP_ENTRY: &str = include_str!("../../../../packaging/linux/cocovm.desktop");

fn desktop_value(key: &str) -> Option<&'static str> {
    DESKTOP_ENTRY
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

#[test]
fn desktop_entry_window_class_matches_app_id() {
    assert_eq!(desktop_value("StartupWMClass"), Some(APP_ID));
}

#[test]
fn desktop_entry_icon_and_exec_use_the_binary_name() {
    assert_eq!(desktop_value("Icon"), Some(APP_ID));
    assert_eq!(desktop_value("Exec"), Some(APP_ID));
}
