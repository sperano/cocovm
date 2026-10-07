//! The context menu's "Show config in …" action: hand a machine's
//! definition file to the host's file manager so the user can find it on
//! disk. The menu label names the file manager the way the platform does
//! (Finder, File Explorer), and the command behind it differs per host.

use std::path::Path;
use std::process::Command;

use super::{ManagerApp, NO_CONFIG_DIR};
use crate::machine_def;

/// What the platform calls its file manager, for the menu label.
#[cfg(target_os = "macos")]
pub(crate) const FILE_MANAGER_NAME: &str = "Finder";
#[cfg(target_os = "windows")]
pub(crate) const FILE_MANAGER_NAME: &str = "File Explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const FILE_MANAGER_NAME: &str = "File Manager";

/// The single-row context menu item that reveals the definition file.
#[cfg(target_os = "macos")]
pub(crate) const SHOW_CONFIG_LABEL: &str = "Show config in Finder";
#[cfg(target_os = "windows")]
pub(crate) const SHOW_CONFIG_LABEL: &str = "Show config in File Explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const SHOW_CONFIG_LABEL: &str = "Show config in File Manager";

impl ManagerApp {
    /// Reveal row `i`'s definition file (`<machines_dir>/<slug>.toml`) in
    /// the host file manager. Success leaves the manager untouched (the
    /// right-click invariant: the menu never moves the selection). A failure
    /// goes on the row's `launch_error`, not `save_error`: selecting a new
    /// row reseeds the detail pane and clears `save_error`, so the message
    /// would never be seen. [`Self::select_row_on_error`] then selects the
    /// row, the same exception the lifecycle actions make.
    pub(super) fn reveal_config(&mut self, i: usize) {
        let result = match &self.machines_dir {
            Some(dir) => reveal(&machine_def::def_path(dir, &self.entries[i].slug)),
            None => Err(NO_CONFIG_DIR.to_string()),
        };
        if let Err(e) = result {
            self.entries[i].launch_error = Some(e);
            self.select_row_on_error(i);
        }
    }
}

/// Launch the host file manager on `path` ([`reveal_command`]).
fn reveal(path: &Path) -> Result<(), String> {
    spawn_and_reap(&mut reveal_command(path))
}

/// Spawn a file-manager launcher. It exits as soon as it has handed the
/// request over, so a thread reaps it rather than leaving a zombie behind
/// for the rest of the session. The error names the file manager and the
/// program that failed to start, since the user sees it in the detail pane.
fn spawn_and_reap(command: &mut Command) -> Result<(), String> {
    let mut child = command.spawn().map_err(|e| {
        format!(
            "could not open {FILE_MANAGER_NAME} ({}): {e}",
            command.get_program().to_string_lossy()
        )
    })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The command that shows `path` in the file manager, selected where the
/// platform supports it: `open -R` on macOS and `explorer /select,` on
/// Windows both open the containing folder with the file highlighted.
/// Linux desktops have no portable "select this file" launcher, so there
/// `xdg-open` opens the containing folder instead.
#[cfg(target_os = "macos")]
fn reveal_command(path: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg("-R").arg(path);
    command
}

#[cfg(target_os = "windows")]
fn reveal_command(path: &Path) -> Command {
    use std::os::windows::process::CommandExt;

    // `/select,` and the path form one argument, and Explorer wants the
    // path quoted itself rather than the whole argument, so bypass std's
    // argument quoting.
    let mut command = Command::new("explorer");
    command.raw_arg(format!("/select,\"{}\"", path.display()));
    command
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn reveal_command(path: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(path.parent().unwrap_or(path));
    command
}

#[cfg(test)]
#[path = "reveal_test.rs"]
mod tests;
