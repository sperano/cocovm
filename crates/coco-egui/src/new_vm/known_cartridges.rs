//! The Cartridge/Slot combos' "Known Cartridges" submenu: lets the user
//! pick a cartridge image the asset bundle already ships
//! (`KnownCartridgeROM::bundled_file`) instead of hunting for it with the
//! file dialog.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use coco_core::rom_db::{KNOWN_CARTRIDGE_ROMS, KnownCartridgeROM};
use eframe::egui;

/// Max height of the submenu's scroll area, matching
/// [`egui::style::Spacing::combo_height`]'s own default so it doesn't dwarf
/// the Cartridge combo it hangs off.
const KNOWN_CARTRIDGE_MENU_MAX_HEIGHT: f32 = 200.0;

/// Bundled cartridge images actually present under `dir`, sorted by
/// [`KnownCartridgeROM::name`]. Existence only — file contents are never
/// read here.
fn bundled_cartridges_in(dir: &Path) -> Vec<(&'static KnownCartridgeROM, PathBuf)> {
    let mut found: Vec<_> = KNOWN_CARTRIDGE_ROMS
        .iter()
        .filter_map(|known| {
            let path = dir.join(known.bundled_file?);
            path.is_file().then_some((known, path))
        })
        .collect();
    found.sort_by_key(|(known, _)| (known.name, known.year, known.variant));
    found
}

/// Submenu row text: the cartridge name, plus the dump-variant tag that
/// distinguishes otherwise identical names ("Color Scripsit [alt]").
fn row_label(known: &KnownCartridgeROM) -> String {
    match known.variant {
        Some(variant) => format!("{} [{variant}]", known.name),
        None => known.name.to_string(),
    }
}

/// Submenu row hover text — the metadata the label leaves out, e.g.
/// "1983 · Tandy · 26-3149". Empty when the manifest has none.
fn row_details(known: &KnownCartridgeROM) -> String {
    let year = known.year.map(|year| year.to_string());
    [year.as_deref(), known.vendor, known.catalog]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// [`bundled_cartridges_in`] against the installed asset bundle
/// (`crate::paths::cartridges_dir`), computed once per run — assets install
/// at startup, so a mid-run download isn't picked up. Empty when the bundle
/// has no `cartridges/` directory.
pub(super) fn bundled_cartridges() -> &'static [(&'static KnownCartridgeROM, PathBuf)] {
    static CACHE: OnceLock<Vec<(&'static KnownCartridgeROM, PathBuf)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        crate::paths::cartridges_dir()
            .map(|dir| bundled_cartridges_in(&dir))
            .unwrap_or_default()
    })
}

/// The Cartridge/Slot combos' "Known Cartridges" submenu: one selectable
/// row per bundled cartridge image, picking `path` exactly as if the file
/// dialog had returned it. Renders nothing when the bundle has none.
pub(super) fn known_cartridge_submenu(
    ui: &mut egui::Ui,
    current: Option<&Path>,
    mut set: impl FnMut(PathBuf),
) {
    let entries = bundled_cartridges();
    if entries.is_empty() {
        return;
    }
    ui.menu_button("Known Cartridges", |ui| {
        egui::ScrollArea::vertical()
            .max_height(KNOWN_CARTRIDGE_MENU_MAX_HEIGHT)
            .show(ui, |ui| {
                for (known, path) in entries {
                    let selected = current == Some(path.as_path());
                    let mut response = ui.selectable_label(selected, row_label(known));
                    let details = row_details(known);
                    if !details.is_empty() {
                        response = response.on_hover_text(details);
                    }
                    if response.clicked() {
                        set(path.clone());
                        // egui's SubMenu forwards this close to the enclosing ComboBox popup.
                        ui.close();
                    }
                }
            });
    });
}

#[cfg(test)]
#[path = "known_cartridges_test.rs"]
mod tests;
