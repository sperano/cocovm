//! The Machine menu's DriveWire submenu.

use crate::*;

const GMC_BECKER_CONFLICT_HINT: &str =
    "The Games Master Cartridge sound port also uses $FF41. Remove it before enabling Becker.";

fn becker_toggle_enabled(becker_enabled: bool, cart: &coco_core::cart::Cart) -> bool {
    becker_enabled || !cart.contains_games_master()
}

impl CocoApp {
    /// The Machine menu's DriveWire submenu: the virtual serial link
    /// and the four disk images it serves.
    pub(super) fn drivewire_menu_ui(&mut self, ui: &mut egui::Ui) {
        let becker_enabled = self.machine.bus.drivewire.is_some();
        let toggle_enabled = becker_toggle_enabled(becker_enabled, &self.machine.bus.cart);
        if ui
            .add_enabled(
                toggle_enabled,
                egui::Button::selectable(becker_enabled, "Enable Becker port ($FF41/$FF42)"),
            )
            .on_disabled_hover_text(GMC_BECKER_CONFLICT_HINT)
            .clicked()
        {
            if becker_enabled {
                self.disable_drivewire();
            } else {
                self.enable_drivewire(false);
            }
        }
        if becker_enabled {
            ui.separator();
            if let Some(ref mut dw) = self.machine.bus.drivewire {
                let mut hdbdos = dw.hdbdos_mode();
                if ui.checkbox(&mut hdbdos, "HDB-DOS mode").changed() {
                    dw.set_hdbdos_mode(hdbdos);
                }
            }
            ui.separator();
            for drive in 0..drivewire::DRIVE_COUNT {
                if ui.button(format!("Mount DW{drive}…")).clicked() {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Disk image", &["dsk", "os9", "img", "vhd"])
                        .pick_file()
                    {
                        self.insert_dw_disk(drive, path);
                    }
                }
                let label = match &self.dw_paths[drive] {
                    Some(p) => format!(
                        "Eject DW{drive} ({})",
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                    ),
                    None => format!("Eject DW{drive}"),
                };
                let mounted = self.dw_paths[drive].is_some();
                if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                    self.eject_dw_disk(drive);
                    ui.close();
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "drivewire_test.rs"]
mod tests;
