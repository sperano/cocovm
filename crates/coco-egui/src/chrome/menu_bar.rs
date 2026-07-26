use crate::*;

impl CocoApp {
    /// The menu bar and all of its menus.
    pub(crate) fn menu_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Machine", |ui| self.machine_menu_ui(ui));
                ui.menu_button("Keyboard", |ui| self.keyboard_menu_ui(ui));
                ui.menu_button("View", |ui| self.view_menu_ui(ui));
                ui.menu_button("Joysticks", |ui| self.joysticks.menu_ui(ui));
                ui.menu_button("Sound", |ui| self.audio.menu_ui(ui));
                ui.menu_button("Help", |ui| self.help_menu_ui(ui));
            });
        });
    }

    /// The Machine menu: cartridges, the MultiPak and its slots, disk
    /// and VHD drives, DriveWire, the cassette deck, and print capture.
    fn machine_menu_ui(&mut self, ui: &mut egui::Ui) {
        let new_button = egui::Button::new("New…")
            .shortcut_text(ui.ctx().format_shortcut(&new_vm::NEW_MACHINE_SHORTCUT));
        if ui.add(new_button).clicked() {
            self.new_vm.open_with(self.machine.config, self.aspect_correct, self.kb_mode);
            ui.close();
        }
        ui.separator();
        let run_label = if self.running { "Pause" } else { "Run" };
        if ui.button(run_label).clicked() {
            self.running = !self.running;
            ui.close();
        }
        if ui.button("Reset").clicked() {
            self.machine.reset();
            ui.close();
        }
        ui.separator();
        self.draw_save_state_menu(ui);
        ui.separator();
        self.machine_cartridge_items(ui);
        ui.separator();
        ui.menu_button("MultiPak Interface", |ui| self.mpi_menu_ui(ui));
        ui.separator();
        ui.menu_button("Deluxe RS-232 Pak", |ui| self.rs232_menu_ui(ui));
        ui.separator();
        self.machine_rtc_items(ui);
        ui.separator();
        self.machine_disk_items(ui);
        ui.separator();
        self.machine_vhd_items(ui);
        ui.separator();
        ui.menu_button("DriveWire", |ui| self.drivewire_menu_ui(ui));
        ui.separator();
        self.machine_tape_items(ui);
        ui.separator();
        self.machine_print_items(ui);
    }

    /// Cartridges plugged straight into the port, which only makes
    /// sense with no MultiPak installed — with one, they go in its slots.
    fn machine_cartridge_items(&mut self, ui: &mut egui::Ui) {
        let direct_port = self.mpi.is_none();
        if ui
            .add_enabled(direct_port, egui::Button::new("Insert Cartridge…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("ROM Pak", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.insert_cartridge(path);
            }
        }
        if ui
            .add_enabled(direct_port, egui::Button::new("Insert Games Master…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Games Master ROM", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.insert_gmc(path);
            }
        }
        if ui
            .add_enabled(direct_port, egui::Button::new("Insert Orchestra-90…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Orchestra-90 ROM", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.insert_orch90(path);
            }
        }
        let inserted = direct_port && self.cart_path.is_some();
        if ui
            .add_enabled(inserted, egui::Button::new("Eject Cartridge"))
            .clicked()
        {
            self.eject_cartridge();
            ui.close();
        }
        ui.checkbox(&mut self.autostart_cart, "Auto-start cartridge");
        if ui
            .add_enabled(direct_port, egui::Button::new("Insert Sound/Speech Cartridge"))
            .clicked()
        {
            self.insert_ssc();
            ui.close();
        }
    }

    /// The Disto real-time clock plugged straight into the port.
    fn machine_rtc_items(&mut self, ui: &mut egui::Ui) {
        let direct_port = self.mpi.is_none();
        if ui
            .add_enabled(
                direct_port && !self.rtc_direct,
                egui::Button::new("Insert Disto RTC"),
            )
            .clicked()
        {
            self.insert_rtc();
            ui.close();
        }
        if ui
            .add_enabled(self.rtc_direct, egui::Button::new("Eject Disto RTC"))
            .clicked()
        {
            self.eject_rtc();
            ui.close();
        }
        let rtc_present = self.machine.bus.cart.as_disto_rtc().is_some();
        if ui
            .add_enabled(rtc_present, egui::Button::new("Sync RTC to Host Clock"))
            .clicked()
        {
            self.sync_rtc_to_host();
            ui.close();
        }
    }

    /// The FD-502 floppy drives: insert, format blank, and eject.
    fn machine_disk_items(&mut self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            if ui.button(format!("Insert Disk in Drive {drive}…")).clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Disk image", &["dsk", "jvc", "os9"])
                    .pick_file()
                {
                    self.request_insert_disk(drive, path);
                }
            }
            if ui.button(format!("New Blank Disk in Drive {drive}…")).clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Disk image", &["dsk"])
                    .set_file_name("untitled.dsk")
                    .save_file()
                {
                    self.request_new_blank_disk(drive, path);
                }
            }
            let label = match &self.disk_paths[drive] {
                Some(p) => format!(
                    "Eject Drive {drive} ({})",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
                None => format!("Eject Drive {drive}"),
            };
            let mounted = self.disk_paths[drive].is_some();
            if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                self.eject_disk(drive);
                ui.close();
            }
        }
    }

    /// The virtual hard disk drives: insert and eject.
    fn machine_vhd_items(&mut self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            if ui.button(format!("Insert VHD {drive}…")).clicked() {
                ui.close();
                if let Some(path) =
                    rfd::FileDialog::new().add_filter("VHD image", &["vhd"]).pick_file()
                {
                    self.insert_vhd(drive, path);
                }
            }
            let label = match &self.vhd_paths[drive] {
                Some(p) => format!(
                    "Eject VHD {drive} ({})",
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                ),
                None => format!("Eject VHD {drive}"),
            };
            let mounted = self.vhd_paths[drive].is_some();
            if ui.add_enabled(mounted, egui::Button::new(label)).clicked() {
                self.eject_vhd(drive);
                ui.close();
            }
        }
    }

    /// The cassette deck: insert, create, rewind, and eject a tape.
    fn machine_tape_items(&mut self, ui: &mut egui::Ui) {
        if ui.button("Insert Tape…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Cassette image", &["cas", "wav"])
                .pick_file()
            {
                self.insert_tape(path);
            }
        }
        if ui.button("New Tape…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Cassette image", &["cas"])
                .set_file_name("untitled.cas")
                .save_file()
            {
                self.new_tape(path);
            }
        }
        let tape_mounted = self.tape_path.is_some();
        if ui
            .add_enabled(tape_mounted, egui::Button::new("Rewind Tape"))
            .clicked()
        {
            self.machine.bus.cassette.rewind();
            ui.close();
        }
        let label = match &self.tape_path {
            Some(p) => format!(
                "Eject Tape ({})",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ),
            None => "Eject Tape".to_string(),
        };
        if ui.add_enabled(tape_mounted, egui::Button::new(label)).clicked() {
            self.eject_tape();
            ui.close();
        }
        ui.checkbox(&mut self.save_tape_wav, "Also save tape audio (.wav)");
    }

    /// Bit-banger print capture to a host text file.
    fn machine_print_items(&mut self, ui: &mut egui::Ui) {
        let capturing = self.print_capture_path.is_some();
        if ui
            .add_enabled(!capturing, egui::Button::new("Start Print Capture…"))
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Text file", &["txt"])
                .set_file_name("printout.txt")
                .save_file()
            {
                self.start_print_capture(path);
            }
        }
        let label = match &self.print_capture_path {
            Some(p) => format!(
                "Stop Print Capture ({})",
                p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            ),
            None => "Stop Print Capture".to_string(),
        };
        if ui
            .add_enabled(capturing, egui::Button::new(label))
            .clicked()
        {
            self.stop_print_capture();
            ui.close();
        }
        ui.checkbox(&mut self.print_capture_lf, "Translate CR to LF")
            .on_hover_text(
                "Rewrite the CoCo's CR line endings as LF so the capture reads as \
                 normal text. Takes effect when a capture starts.",
            );
    }


    /// The Keyboard menu: positional/symbolic mode and the key map.
    fn keyboard_menu_ui(&mut self, ui: &mut egui::Ui) {
        for mode in [KbMode::Positional, KbMode::Symbolic] {
            if ui.selectable_label(self.kb_mode == mode, mode.label()).clicked() {
                self.set_mode(mode);
            }
        }
        ui.separator();
        if ui.button("Key layout (F10)").clicked() {
            self.show_kbd_help = !self.show_kbd_help;
            ui.close();
        }
    }

    /// The View menu: scaling, aspect correction, and the optional
    /// Orchestra-90 level meters and monitor type.
    fn view_menu_ui(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.aspect_correct, "4:3 aspect (F9)");
        ui.separator();
        ui.checkbox(&mut self.debugger.open, "Debugger (F11)");
        ui.separator();
        let mut paper_open = self.paper_window.open;
        if ui.checkbox(&mut paper_open, "Printer Paper").changed() {
            self.toggle_paper_window();
        }
        // Only meaningful with an Orchestra-90 cartridge actually inserted
        // (direct port or in an MPI slot) — `as_orch90` searches both.
        let orch90_present = self.machine.bus.cart.as_orch90().is_some();
        ui.add_enabled(
            orch90_present,
            egui::Checkbox::new(&mut self.show_orch90, "Orchestra-90 Levels"),
        );
        ui.separator();
        // Swapping the monitor cable doesn't erase machine state,
        // so this takes effect live rather than requiring a
        // power cycle.
        for (mt, label) in [
            (MonitorType::RGB, "RGB monitor"),
            (MonitorType::Composite, "Composite monitor"),
        ] {
            if ui
                .selectable_label(self.machine.bus.gime.monitor == mt, label)
                .clicked()
            {
                self.machine.bus.gime.monitor = mt;
            }
        }
    }

    /// The Help menu.
    fn help_menu_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button("About").clicked() {
            self.show_about = !self.show_about;
            ui.close();
        }
    }

    /// The Machine menu's MultiPak Interface submenu: what is in each
    /// slot, and which slot the switch selects.
    fn mpi_menu_ui(&mut self, ui: &mut egui::Ui) {
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
                let slot_label = match self.mpi.as_ref().map(|m| &m.slots[slot]) {
                    Some(MPISlot::ROMPak(p)) => format!(
                        "Slot {} ({})",
                        slot + 1,
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                    ),
                    Some(MPISlot::FD502) => format!("Slot {} (FD-502)", slot + 1),
                    Some(MPISlot::DistoRTC) => {
                        format!("Slot {} (Disto RTC)", slot + 1)
                    }
                    Some(MPISlot::Gmc(p)) => format!(
                        "Slot {} (GMC: {})",
                        slot + 1,
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                    ),
                    Some(MPISlot::Orch90(p)) => format!(
                        "Slot {} (Orchestra-90: {})",
                        slot + 1,
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                    ),
                    Some(MPISlot::Ssc) => {
                        format!("Slot {} (Sound/Speech)", slot + 1)
                    }
                    _ => format!("Slot {}", slot + 1),
                };
                ui.menu_button(slot_label, |ui| self.mpi_slot_menu_ui(ui, slot));
            }
            ui.separator();
            ui.menu_button("Switch", |ui| self.mpi_switch_menu_ui(ui));
        }
    }

    /// The Machine menu's Deluxe RS-232 Pak submenu.
    fn rs232_menu_ui(&mut self, ui: &mut egui::Ui) {
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
        // Re-read instead of reusing `installed`: a Remove
        // click above already cleared `self.rs232` this same
        // frame.
        let current = match &self.rs232 {
            Some(Rs232Endpoint::Loopback) => Some(Rs232EndpointKind::Loopback),
            Some(Rs232Endpoint::Tcp(_)) => Some(Rs232EndpointKind::Tcp),
            Some(Rs232Endpoint::Pty(_)) => Some(Rs232EndpointKind::Pty),
            None => None,
        };
        if let Some(current) = current {
            ui.separator();
            ui.label("Wire the serial line to:");
            if ui
                .selectable_label(
                    current == Rs232EndpointKind::Loopback,
                    "Loopback",
                )
                .clicked()
            {
                self.rs232_set_endpoint(Rs232EndpointKind::Loopback);
            }
            let tcp_label = match &self.rs232 {
                Some(Rs232Endpoint::Tcp(addr)) => format!("TCP ({addr})"),
                _ => "TCP".to_string(),
            };
            if ui
                .selectable_label(current == Rs232EndpointKind::Tcp, tcp_label)
                .clicked()
            {
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
            if ui
                .selectable_label(current == Rs232EndpointKind::Pty, pty_label)
                .clicked()
            {
                self.rs232_set_endpoint(Rs232EndpointKind::Pty);
            }
            if let Some(pak) = self.machine.bus.cart.as_deluxe_rs232() {
                ui.separator();
                ui.label(format!(
                    "TX {} bytes / RX {} bytes",
                    pak.tx_bytes(),
                    pak.rx_bytes()
                ));
            }
        }
    }

    /// The Machine menu's DriveWire submenu: the virtual serial link
    /// and the four disk images it serves.
    fn drivewire_menu_ui(&mut self, ui: &mut egui::Ui) {
        let becker_enabled = self.machine.bus.drivewire.is_some();
        if ui
            .selectable_label(becker_enabled, "Enable Becker port ($FF41/$FF42)")
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

    /// One MultiPak slot's submenu: what can go into it, and what
    /// is in it now.
    fn mpi_slot_menu_ui(&mut self, ui: &mut egui::Ui, slot: usize) {
        if ui.button("Insert ROM Pak…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("ROM Pak", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.mpi_insert_rompak(slot, path);
            }
        }
        if ui.button("Insert Games Master…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Games Master ROM", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.mpi_insert_gmc(slot, path);
            }
        }
        if ui.button("Insert Orchestra-90…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Orchestra-90 ROM", &["rom", "ccc", "bin"])
                .pick_file()
            {
                self.mpi_insert_orch90(slot, path);
            }
        }
        // An FD-502 already installed elsewhere can't also go here
        // — the emulated latch only ever models one controller.
        let fd502_here = matches!(
            self.mpi.as_ref().map(|m| &m.slots[slot]),
            Some(MPISlot::FD502)
        );
        let fd502_elsewhere =
            self.machine.bus.cart.as_disk_cart().is_some() && !fd502_here;
        if ui
            .add_enabled(
                !fd502_elsewhere,
                egui::Button::new("Insert FD-502"),
            )
            .clicked()
        {
            self.mpi_insert_fd502(slot);
            ui.close();
        }
        // Same one-per-machine rule as the FD-502:
        // two RTCs would shadow each other at $FF50.
        let rtc_here = matches!(
            self.mpi.as_ref().map(|m| &m.slots[slot]),
            Some(MPISlot::DistoRTC)
        );
        let rtc_elsewhere = self
            .machine
            .bus
            .cart
            .as_disto_rtc()
            .is_some()
            && !rtc_here;
        if ui
            .add_enabled(
                !rtc_elsewhere,
                egui::Button::new("Insert Disto RTC"),
            )
            .clicked()
        {
            self.mpi_insert_rtc(slot);
            ui.close();
        }
        if ui.button("Insert Sound/Speech").clicked() {
            self.mpi_insert_ssc(slot);
            ui.close();
        }
        let occupied = !matches!(
            self.mpi.as_ref().map(|m| &m.slots[slot]),
            Some(MPISlot::Empty)
        );
        if ui
            .add_enabled(occupied, egui::Button::new("Eject"))
            .clicked()
        {
            self.mpi_eject_slot(slot);
            ui.close();
        }
    }
}
