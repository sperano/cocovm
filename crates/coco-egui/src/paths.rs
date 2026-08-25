//! Per-user application directories.
//!
//! Uses `etcetera`'s `choose_app_strategy`, which follows the XDG Base
//! Directory spec on Linux *and* macOS (`~/.config/cocovm`,
//! `~/.local/share/cocovm`, …) and the native convention on Windows
//! (`%APPDATA%\spe\cocovm\…`).
//! macOS deliberately gets XDG rather than `~/Library/Application Support`
//! so the config stays where terminal users expect to edit and version it.

use std::path::PathBuf;

use etcetera::app_strategy::{AppStrategy, AppStrategyArgs};

fn strategy() -> Result<impl AppStrategy, etcetera::HomeDirError> {
    etcetera::choose_app_strategy(AppStrategyArgs {
        top_level_domain: "quebec".to_string(),
        author: "spe".to_string(),
        app_name: "cocovm".to_string(),
    })
}

/// Directory for user configuration (`~/.config/cocovm` on Linux/macOS).
/// Returns `None` if no home directory can be determined; not created — callers must
/// `fs::create_dir_all`.
pub fn config_dir() -> Option<PathBuf> {
    strategy().ok().map(|s| s.config_dir())
}

/// Directory for user data such as saved disks or state
/// (`~/.local/share/cocovm` on Linux/macOS).
pub fn data_dir() -> Option<PathBuf> {
    strategy().ok().map(|s| s.data_dir())
}

/// Directory for ROM assets
/// (`~/.local/share/cocovm/roms` on Linux/macOS).
pub fn roms_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("roms"))
}

/// Directory for image assets
/// (`~/.local/share/cocovm/images` on Linux/macOS).
pub fn images_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("images"))
}

#[cfg(test)]
#[path = "paths_test.rs"]
mod tests;
