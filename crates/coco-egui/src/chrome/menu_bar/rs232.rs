//! The Machine menu's Deluxe RS-232 Pak submenu.

use crate::*;

impl CocoApp {
    /// The Machine menu's Deluxe RS-232 Pak submenu.
    pub(super) fn rs232_menu_ui(&mut self, ui: &mut egui::Ui) {
        let direct_port = self.mpi.is_none();
        let installed = self.rs232.is_some();
        // Like "Insert Cartridge…": the pak plugs straight
        // into the port, so an installed MPI blocks it.
        if ui
            .add_enabled(
                direct_port && !installed,
                egui::Button::new("Insert Deluxe RS-232 Pak"),
            )
            .clicked()
        {
            self.insert_rs232();
            ui.close();
        }
        if ui
            .add_enabled(installed, egui::Button::new("Remove Deluxe RS-232 Pak"))
            .clicked()
        {
            self.eject_cartridge();
            ui.close();
        }
        // Re-read instead of reusing `installed`: a Remove click above
        // already cleared `self.rs232` this same frame.
        if self.rs232.is_some() {
            ui.separator();
            self.rs232_endpoint_items(ui);
            self.rs232_byte_counters(ui);
        }
    }

    /// Which host backend the pak's serial line is wired to.
    fn rs232_endpoint_items(&mut self, ui: &mut egui::Ui) {
        let current = self.rs232.as_ref().map(Rs232Endpoint::kind);
        let selected = |kind| current == Some(kind);

        ui.label("Wire the serial line to:");
        if ui.selectable_label(selected(Rs232EndpointKind::Loopback), "Loopback").clicked() {
            self.rs232_set_endpoint(Rs232EndpointKind::Loopback);
        }

        let tcp_label = match &self.rs232 {
            Some(Rs232Endpoint::Tcp(addr)) => format!("TCP ({addr})"),
            _ => "TCP".to_string(),
        };
        if ui.selectable_label(selected(Rs232EndpointKind::Tcp), tcp_label).clicked() {
            self.rs232_set_endpoint(Rs232EndpointKind::Tcp);
        }
        ui.horizontal(|ui| {
            ui.label("Listen address:");
            ui.text_edit_singleline(&mut self.rs232_tcp_addr);
        });

        let pty_label = match &self.rs232 {
            Some(Rs232Endpoint::Pty(path)) => format!("PTY ({path})"),
            _ => "PTY".to_string(),
        };
        if ui.selectable_label(selected(Rs232EndpointKind::Pty), pty_label).clicked() {
            self.rs232_set_endpoint(Rs232EndpointKind::Pty);
        }
    }

    fn rs232_byte_counters(&mut self, ui: &mut egui::Ui) {
        let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() else {
            return;
        };
        ui.separator();
        ui.label(format!("TX {} bytes / RX {} bytes", pak.tx_bytes(), pak.rx_bytes()));
    }
}
