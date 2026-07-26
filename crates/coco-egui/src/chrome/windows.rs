use crate::*;

impl CocoApp {
    /// Every optional window and modal dialog drawn over the display.
    pub(crate) fn windows_ui(&mut self, ctx: &egui::Context) {
        if self.show_kbd_help {
            let symbolic = self.kb_mode == KbMode::Symbolic;
            kbd_help::window(ctx, &mut self.show_kbd_help, symbolic);
        }
        if self.show_about {
            about::window(ctx, &mut self.show_about);
        }
        if self.show_orch90
            && let Some(orch90) = self.machine.bus.cart.as_orch90()
        {
            orch90_meters::window(ctx, &mut self.show_orch90, orch90.left(), orch90.right());
        }
        self.debugger.windows_ui(ctx, &mut self.machine, &mut self.running);
        self.new_vm_dialog_ui(ctx);
        if let Some(err) = self.paper_window.ui(ctx) {
            self.cart_error = Some(err);
        }
        self.disk_controller_prompt_ui(ctx);
        self.cart_error_ui(ctx);
    }

    /// The "New…" dialog, and the machine it builds when the user confirms.
    fn new_vm_dialog_ui(&mut self, ctx: &egui::Context) {
        let new_vm::NewVmAction::Create(spec) = self.new_vm.show(ctx) else {
            return;
        };
        if let Err(e) = self.create_vm(spec.config, ctx) {
            self.new_vm.error = Some(e);
            return;
        }
        // The new window's starting UI preferences, from the form's
        // Display/Keyboard rows; F9/F12 keep toggling them live afterwards.
        self.aspect_correct = spec.aspect_correct;
        self.kb_mode = spec.kb_mode;
        // The machine booted; cartridge/media problems (e.g. missing
        // disk11.rom, unreadable image) are reported like a menu insert,
        // not as a create failure.
        let has_drives = spec.has_drives();
        let new_vm::NewMachineSpec { cartridge, mpi_slots, disks, tape, vhds, .. } = *spec;
        self.mount_dialog_cartridge(cartridge, mpi_slots, disks, has_drives);
        self.mount_dialog_tape(tape);
        self.mount_dialog_vhds(vhds);
        self.new_vm.close();
    }

    /// Whatever the "New…" dialog's Cartridge row selected, plus the floppies
    /// that come with an FD-502 — bare, or in a MultiPak slot.
    fn mount_dialog_cartridge(
        &mut self,
        cartridge: new_vm::CartridgeChoice,
        mpi_slots: [new_vm::SlotChoice; MPI_SLOT_COUNT],
        disks: [new_vm::MediaChoice; UI_DRIVES],
        has_drives: bool,
    ) {
        match cartridge {
            new_vm::CartridgeChoice::None => {}
            new_vm::CartridgeChoice::RomPak(path) => self.insert_cartridge(path),
            new_vm::CartridgeChoice::RTC => self.insert_rtc(),
            new_vm::CartridgeChoice::FD502 => {
                if let Err(e) = self.ensure_disk_controller() {
                    self.cart_error = Some(e);
                } else {
                    self.mount_dialog_disks(disks);
                }
            }
            new_vm::CartridgeChoice::MPI => {
                self.insert_multipak();
                for (slot, choice) in mpi_slots.into_iter().enumerate() {
                    match choice {
                        new_vm::SlotChoice::FD502 => self.mpi_insert_fd502(slot),
                        new_vm::SlotChoice::RomPak(path) => self.mpi_insert_rompak(slot, path),
                        new_vm::SlotChoice::RTC => self.mpi_insert_rtc(slot),
                        new_vm::SlotChoice::Empty => {}
                    }
                }
                if has_drives {
                    self.mount_dialog_disks(disks);
                }
            }
        }
    }

    /// The "New…" dialog's cassette row.
    fn mount_dialog_tape(&mut self, tape: new_vm::MediaChoice) {
        match tape {
            new_vm::MediaChoice::File(path) => self.insert_tape(path),
            new_vm::MediaChoice::Blank(Some(path)) => self.new_tape(path),
            new_vm::MediaChoice::None | new_vm::MediaChoice::Blank(None) => {}
        }
    }

    /// The "New…" dialog's virtual-hard-disk rows.
    fn mount_dialog_vhds(&mut self, vhds: [new_vm::MediaChoice; UI_DRIVES]) {
        for (drive, choice) in vhds.into_iter().enumerate() {
            match choice {
                new_vm::MediaChoice::File(path) => self.insert_vhd(drive, path),
                new_vm::MediaChoice::Blank(Some(path)) => self.new_vhd(drive, path),
                new_vm::MediaChoice::None | new_vm::MediaChoice::Blank(None) => {}
            }
        }
    }

    /// Confirmation for a disk action that needs an FD-502 the machine
    /// doesn't have yet — installing one cold-restarts the machine.
    fn disk_controller_prompt_ui(&mut self, ctx: &egui::Context) {
        if self.pending_disk_action.is_none() {
            return;
        }
        // Match the dialog body to the button font (egui's default body
        // text is a touch smaller) and give the text room.
        let font = ctx.style().text_styles[&egui::TextStyle::Button].size;
        const DIALOG_MARGIN: i8 = 16;
        egui::Window::new(window_title(ctx, "Insert disk controller?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::NONE.inner_margin(DIALOG_MARGIN).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "The FD-502 disk controller isn't installed yet. Installing \
                             it swaps the cartridge and cold-restarts the machine — any \
                             unsaved work in memory will be lost.",
                        )
                        .size(font),
                    );
                    ui.add_space(DIALOG_MARGIN as f32);
                    ui.horizontal(|ui| {
                        // Roomier buttons: pad text away from the button edge.
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 6.0);
                        if ui.button("Insert & Restart").clicked() {
                            match self.pending_disk_action.take() {
                                Some(PendingDiskAction::Insert { drive, path }) => {
                                    self.insert_disk(drive, path)
                                }
                                Some(PendingDiskAction::NewBlank { drive, path }) => {
                                    self.new_blank_disk(drive, path)
                                }
                                None => {}
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_disk_action = None;
                        }
                    });
                });
            });
    }

    /// Dismissible banner for the last failed cartridge or media load.
    fn cart_error_ui(&mut self, ctx: &egui::Context) {
        let Some(err) = self.cart_error.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(window_title(ctx, "Cartridge Error"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(err);
                if ui.button("OK").clicked() {
                    self.cart_error = None;
                }
            });
        if !open {
            self.cart_error = None;
        }
    }
}
