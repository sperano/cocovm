//! The Machine menu's MultiPak Interface submenu and the per-slot and
//! switch submenus nested inside it.

use crate::*;

impl CocoApp {
    /// The Machine menu's MultiPak Interface submenu: what is in each
    /// slot, and which slot the switch selects.
    pub(super) fn mpi_menu_ui(&mut self, ui: &mut egui::Ui) {
        let installed = self.mpi.is_some();
        if ui
            .add_enabled(!installed, egui::Button::new("Insert MultiPak"))
            .clicked()
        {
            self.insert_multipak();
            ui.close();
        }
        if ui
            .add_enabled(installed, egui::Button::new("Remove MultiPak"))
            .clicked()
        {
            self.eject_multipak();
            ui.close();
        }
        if installed {
            ui.separator();
            for slot in 0..MPI_SLOT_COUNT {
                let label = slot_menu_label(slot, self.mpi_slot(slot));
                ui.menu_button(label, |ui| self.mpi_slot_menu_ui(ui, slot));
            }
            ui.separator();
            ui.menu_button("Switch", |ui| self.mpi_switch_menu_ui(ui));
        }
    }

    /// One MultiPak slot's submenu: what can go into it, and what
    /// is in it now.
    fn mpi_slot_menu_ui(&mut self, ui: &mut egui::Ui, slot: usize) {
        self.mpi_slot_rom_items(ui, slot);
        self.mpi_slot_fd502_item(ui, slot);
        self.mpi_slot_rtc_item(ui, slot);
        if ui.button("Insert Sound/Speech").clicked() {
            self.mpi_insert_ssc(slot);
            ui.close();
        }
        if ui
            .add_enabled(self.mpi_slot_occupied(slot), egui::Button::new("Eject"))
            .clicked()
        {
            self.mpi_eject_slot(slot);
            ui.close();
        }
    }

    /// The three ROM-image picks a slot accepts; any number of slots may hold
    /// one, so none of them is gated.
    fn mpi_slot_rom_items(&mut self, ui: &mut egui::Ui, slot: usize) {
        if let Some(path) = rom_pick(ui, "Insert ROM Pak…", "ROM Pak") {
            self.mpi_insert_rompak(slot, path);
        }
        if let Some(path) = rom_pick(ui, "Insert Games Master…", "Games Master ROM") {
            self.mpi_insert_gmc(slot, path);
        }
        if let Some(path) = rom_pick(ui, "Insert Orchestra-90…", "Orchestra-90 ROM") {
            self.mpi_insert_orch90(slot, path);
        }
    }

    /// An FD-502 already installed elsewhere can't also go here — the
    /// emulated latch only ever models one controller.
    fn mpi_slot_fd502_item(&mut self, ui: &mut egui::Ui, slot: usize) {
        let here = matches!(self.mpi_slot(slot), Some(MPISlot::FD502));
        let elsewhere = self.machine.bus.cart.as_disk_cart().is_some() && !here;
        if ui.add_enabled(!elsewhere, egui::Button::new("Insert FD-502")).clicked() {
            self.mpi_insert_fd502(slot);
            ui.close();
        }
    }

    /// Same one-per-machine rule as the FD-502: two RTCs would shadow each
    /// other at `$FF50`.
    fn mpi_slot_rtc_item(&mut self, ui: &mut egui::Ui, slot: usize) {
        let here = matches!(self.mpi_slot(slot), Some(MPISlot::DistoRTC));
        let elsewhere = self.machine.bus.cart.as_disto_rtc().is_some() && !here;
        if ui.add_enabled(!elsewhere, egui::Button::new("Insert Disto RTC")).clicked() {
            self.mpi_insert_rtc(slot);
            ui.close();
        }
    }

    fn mpi_slot(&self, slot: usize) -> Option<&MPISlot> {
        self.mpi.as_ref().map(|m| &m.slots[slot])
    }

    fn mpi_slot_occupied(&self, slot: usize) -> bool {
        !matches!(self.mpi_slot(slot), Some(MPISlot::Empty))
    }

    /// The MultiPak submenu picking which slot the physical switch selects.
    fn mpi_switch_menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Selects the power-on SCS/CTS slot. A running program's own \
             $FF7F write overrides it until the next reset.",
        );
        let current = self.mpi.as_ref().map_or(0, |m| m.switch);
        for slot in 0..MPI_SLOT_COUNT {
            if ui
                .selectable_label(current == slot, format!("Slot {}", slot + 1))
                .clicked()
            {
                self.mpi_set_switch(slot);
            }
        }
    }
}

/// A menu item that opens a ROM-image file dialog when clicked, returning the
/// picked path. The menu closes on the click itself, as everywhere else.
fn rom_pick(ui: &mut egui::Ui, label: &str, filter: &str) -> Option<PathBuf> {
    const ROM_EXTENSIONS: &[&str] = &["rom", "ccc", "bin"];
    if !ui.button(label).clicked() {
        return None;
    }
    ui.close();
    rfd::FileDialog::new().add_filter(filter, ROM_EXTENSIONS).pick_file()
}

/// A slot's menu-bar entry, naming what is currently in it.
fn slot_menu_label(slot: usize, contents: Option<&MPISlot>) -> String {
    let number = slot + 1;
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
    match contents {
        Some(MPISlot::ROMPak(p)) => format!("Slot {number} ({})", name(p)),
        Some(MPISlot::FD502) => format!("Slot {number} (FD-502)"),
        Some(MPISlot::DistoRTC) => format!("Slot {number} (Disto RTC)"),
        Some(MPISlot::Gmc(p)) => format!("Slot {number} (GMC: {})", name(p)),
        Some(MPISlot::Orch90(p)) => format!("Slot {number} (Orchestra-90: {})", name(p)),
        Some(MPISlot::Ssc) => format!("Slot {number} (Sound/Speech)"),
        _ => format!("Slot {number}"),
    }
}
