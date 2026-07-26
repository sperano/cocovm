use crate::*;

impl CocoApp {
    /// The status bar: read-only live state, no controls.
    pub(crate) fn status_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(if self.running { "Running" } else { "Paused" });
                ui.separator();
                ui.label(format!("Keyboard: {} (F12)", self.kb_mode.label()));
                self.cart_status(ui);
                self.rs232_status(ui);
                self.mpi_status(ui);
                self.disk_status(ui);
                self.vhd_status(ui);
                self.drivewire_status(ui);
                self.tape_status(ui);
                if let Some(toast) = self.toast_message() {
                    ui.separator();
                    ui.label(toast);
                }
            });
        });
    }

    fn cart_status(&self, ui: &mut egui::Ui) {
        let Some(path) = &self.cart_path else { return };
        ui.separator();
        ui.label(format!("Cart: {}", file_name(path)));
    }

    fn rs232_status(&mut self, ui: &mut egui::Ui) {
        let Some(endpoint) = &self.rs232 else { return };
        ui.separator();
        // ↑/↓ = bytes out to / in from the host endpoint.
        let (tx, rx) = self
            .machine
            .bus
            .cart
            .as_deluxe_rs232()
            .map_or((0, 0), |pak| (pak.tx_bytes(), pak.rx_bytes()));
        ui.label(format!("RS-232 [{}] ↑{tx} ↓{rx}", endpoint.label()));
    }

    fn mpi_status(&self, ui: &mut egui::Ui) {
        let Some(mpi) = &self.mpi else { return };
        ui.separator();
        let slots: Vec<String> = mpi
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| format!("S{}:{}", i + 1, slot_label(slot)))
            .collect();
        ui.label(format!("MPI [{}]", slots.join(" ")));
    }

    fn disk_status(&mut self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            let Some(path) = &self.disk_paths[drive] else {
                continue;
            };
            // "*" = modified in memory; written back on eject/exit.
            let dirty = self
                .machine
                .bus
                .cart
                .as_disk_cart()
                .and_then(|c| c.disk(drive))
                .is_some_and(|d| d.dirty());
            let active = self
                .machine
                .bus
                .cart
                .as_disk_cart()
                .is_some_and(|c| c.drive_active(drive));
            ui.separator();
            drive_activity_light(ui, active);
            ui.label(format!("D{drive}: {}{}", file_name(path), dirty_mark(dirty)));
        }
    }

    fn vhd_status(&self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            let Some(path) = &self.vhd_paths[drive] else {
                continue;
            };
            ui.separator();
            ui.label(format!("VHD{drive}: {}", file_name(path)));
        }
    }

    fn drivewire_status(&self, ui: &mut egui::Ui) {
        let Some(dw) = &self.machine.bus.drivewire else {
            return;
        };
        for drive in 0..drivewire::DRIVE_COUNT {
            let Some(path) = &self.dw_paths[drive] else {
                continue;
            };
            ui.separator();
            ui.label(format!(
                "DW{drive}: {}{}",
                file_name(path),
                dirty_mark(dw.dirty(drive))
            ));
        }
    }

    fn tape_status(&self, ui: &mut egui::Ui) {
        let Some(path) = &self.tape_path else { return };
        let cassette = &self.machine.bus.cassette;
        // The icon reddens while the motor runs (relay closed —
        // CLOAD/CSAVE/MOTOR ON); the counter is the playback position in
        // tape bytes; "*" as for floppies.
        let motor = self.machine.bus.pia1.a.c2_output();
        let (pos, len) = cassette.position();
        ui.separator();
        cassette_activity_light(ui, motor);
        ui.label(format!(
            "Tape: {}{} [{pos}/{len}]",
            file_name(path),
            dirty_mark(cassette.dirty())
        ));
    }
}

/// The file name of a mounted image, for the one-line status readout.
fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
}

/// Trailing "*" marking an image modified in memory but not yet written back.
fn dirty_mark(dirty: bool) -> &'static str {
    if dirty { "*" } else { "" }
}

fn slot_label(slot: &MPISlot) -> String {
    match slot {
        MPISlot::Empty => "-".to_string(),
        MPISlot::ROMPak(p) => file_name(p).to_string(),
        MPISlot::FD502 => "FD-502".to_string(),
        MPISlot::DistoRTC => "RTC".to_string(),
        MPISlot::Gmc(p) => format!("GMC:{}", file_name(p)),
        MPISlot::Orch90(p) => format!("Orchestra-90:{}", file_name(p)),
        MPISlot::Ssc => "SSC".to_string(),
    }
}
