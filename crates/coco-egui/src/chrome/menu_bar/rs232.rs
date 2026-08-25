//! The Machine menu's Deluxe RS-232 Pak submenu.

use crate::*;

impl CocoApp {
    /// The Machine menu's Deluxe RS-232 Pak submenu.
    pub(super) fn rs232_menu_ui(&mut self, ui: &mut egui::Ui) {
        let direct_port = self.mpi.is_none();
        let installed = self.rs232.is_some();
        // The pak plugs straight into the port, so an installed MPI blocks it.
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
        // Re-read instead of reusing `installed`: a Remove click above already cleared it this frame.
        if self.rs232.is_some() {
            ui.separator();
            self.rs232_endpoint_items(ui);
            self.rs232_byte_counters(ui);
        }
    }

    /// Which host backend the pak's serial line is wired to.
    fn rs232_endpoint_items(&mut self, ui: &mut egui::Ui) {
        let current = self.rs232.as_ref().map(RS232Endpoint::kind);
        let selected = |kind| current == Some(kind);

        ui.label("Wire the serial line to:");
        if ui
            .selectable_label(selected(RS232EndpointKind::Loopback), "Loopback")
            .clicked()
        {
            self.rs232_set_endpoint(RS232EndpointKind::Loopback);
        }

        let tcp_label = match &self.rs232 {
            Some(RS232Endpoint::TCP(addr)) => format!("TCP ({addr})"),
            _ => "TCP".to_string(),
        };
        if ui
            .selectable_label(selected(RS232EndpointKind::TCP), tcp_label)
            .clicked()
        {
            self.rs232_set_endpoint(RS232EndpointKind::TCP);
        }
        ui.horizontal(|ui| {
            ui.label("Listen address:");
            ui.text_edit_singleline(&mut self.rs232_tcp_addr);
        });

        #[cfg(unix)]
        {
            let pty_label = match &self.rs232 {
                Some(RS232Endpoint::PTY(path)) => format!("PTY ({path})"),
                _ => "PTY".to_string(),
            };
            if ui
                .selectable_label(selected(RS232EndpointKind::PTY), pty_label)
                .clicked()
            {
                self.rs232_set_endpoint(RS232EndpointKind::PTY);
            }
        }
    }

    fn rs232_byte_counters(&mut self, ui: &mut egui::Ui) {
        let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() else {
            return;
        };
        ui.separator();
        ui.label(format!(
            "TX {} bytes / RX {} bytes",
            pak.tx_bytes(),
            pak.rx_bytes()
        ));
    }
}
