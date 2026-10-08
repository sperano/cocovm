use std::ffi::OsStr;
use std::path::Path;

use super::*;

fn args(command: &Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

#[test]
fn label_names_the_platform_file_manager() {
    assert_eq!(
        SHOW_CONFIG_LABEL,
        format!("Show config in {FILE_MANAGER_NAME}")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn reveal_command_selects_the_file_in_finder() {
    let path = Path::new("/Users/me/machines/dev.toml");
    let command = reveal_command(path);
    assert_eq!(command.get_program(), "open");
    assert_eq!(args(&command), [OsStr::new("-R"), path.as_os_str()]);
}

#[cfg(target_os = "windows")]
#[test]
fn reveal_command_selects_the_file_in_explorer() {
    let path = Path::new(r"C:\Users\me\machines\dev.toml");
    let command = reveal_command(path);
    assert_eq!(command.get_program(), "explorer");
    assert_eq!(
        args(&command),
        [OsStr::new(r#"/select,"C:\Users\me\machines\dev.toml""#)]
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[test]
fn reveal_command_opens_the_containing_folder() {
    let path = Path::new("/home/me/machines/dev.toml");
    let command = reveal_command(path);
    assert_eq!(command.get_program(), "xdg-open");
    assert_eq!(args(&command), [OsStr::new("/home/me/machines")]);
}

#[test]
fn spawn_and_reap_names_the_missing_launcher() {
    const MISSING_LAUNCHER: &str = "cocovm-no-such-launcher";

    let error = spawn_and_reap(&mut Command::new(MISSING_LAUNCHER))
        .expect_err("a launcher that does not exist cannot spawn");
    assert!(error.contains(FILE_MANAGER_NAME), "{error}");
    assert!(error.contains(MISSING_LAUNCHER), "{error}");
}
