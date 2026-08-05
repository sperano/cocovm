use coco_core::joystick::{LEFT, RIGHT};

use crate::*;

impl CocoApp {
    /// The status bar: live state readouts, plus the menus of the three
    /// entries that are also controls ([`Self::keyboard_status`],
    /// [`Self::display_status`], [`Self::tape_status`]).
    ///
    /// Pinned to [`STATUS_BAR_H`] rather than left to size itself around its
    /// content: that constant is what the window-sizing math already
    /// reserves for this row (`manager::vm_windows`), so an exact
    /// height is what keeps the reservation and the rendered bar the same
    /// number. `horizontal_centered` then takes the full panel height, so
    /// icons and labels ride the middle of the bar instead of hugging its
    /// top edge.
    pub(crate) fn status_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(STATUS_BAR_H)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    self.keyboard_status(ui);
                    self.display_status(ui);
                    self.tape_status(ui);
                    self.cart_status(ui);
                    self.joystick_status(ui);
                    self.rs232_status(ui);
                    self.mpi_status(ui);
                    self.disk_status(ui);
                    self.vhd_status(ui);
                    self.drivewire_status(ui);
                    self.printer_status(ui);
                    if let Some(toast) = self.toast_message() {
                        ui.separator();
                        ui.label(toast);
                    }
                });
            });
    }

    /// The keyboard entry — a status-bar entry that is also a control
    /// (like [`Self::display_status`]): icon and mode label
    /// ("Positional"/"Symbolic") are a single click target that pops up the
    /// keyboard menu (`keyboard_menu_ui` — positional/symbolic, then the
    /// key layout window). This entry is the only way in: the menu bar has
    /// no Keyboard menu of its own.
    ///
    /// Naming the mode means the two labels differ in width, so toggling
    /// (F12) nudges every entry to the right — accepted: seeing the active
    /// mode at a glance beats a fixed-width first entry, and the hover text
    /// keeps the device name ("Keyboard input mode") the label drops.
    ///
    /// The label is a frameless button rather than a plain one: `frame(false)`
    /// zeroes the button padding too, so it lines up with the plain labels
    /// of every other entry while still lighting up under the pointer. The
    /// icon is painted, not a widget, so `interact` is what makes its half
    /// of the entry clickable — and since a click sense is also a focus
    /// sense, it needs [`WidgetInfo`] naming it or it joins the tab order
    /// as an unnamed stop.
    ///
    /// [`WidgetInfo`]: egui::WidgetInfo
    fn keyboard_status(&mut self, ui: &mut egui::Ui) {
        let icon = keyboard_icon(ui).interact(egui::Sense::click());
        icon.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Keyboard menu")
        });
        let mode = self.kb_mode.label();
        let entry = (icon | ui.add(egui::Button::new(mode).frame(false))).on_hover_text(format!(
            "Keyboard input mode: {mode} — click for the keyboard menu (F12 toggles)"
        ));
        egui::Popup::menu(&entry)
            // An explicit id, not the union's: that one is the icon's auto
            // id, which is stable only as long as the keyboard stays the
            // bar's first entry (every entry after it is conditional, so
            // mounting a disk mid-session would renumber it and silently
            // drop an open menu).
            .id(ui.id().with("keyboard_menu"))
            // Anchored above the bar rather than left to egui's own
            // flipping: `BOTTOM_START.symmetries()` does include
            // `TOP_START`, but only a popup with a remembered size can be
            // seen not to fit. On its first frame the candidate is
            // zero-height, so it "fits" against the window's bottom edge
            // and paints one clipped frame before snapping up.
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.keyboard_menu_ui(ui));
    }

    /// The display entry — the bar's other entry that is also a control,
    /// built exactly like [`Self::keyboard_status`]: a painted icon
    /// (monitor or TV set, tracking the current [`Display`]) and a short
    /// frameless-button label form one click target that pops up the
    /// display menu (`display_menu_ui` — the choices plus the TV knobs);
    /// like the keyboard entry, this is the only way in — the View menu
    /// doesn't carry the choice. The full device name lives in the hover
    /// text; the label is [`Display::short_label`]. See the keyboard
    /// entry's comments for why the popup gets an explicit id and a
    /// `TOP_START` anchor.
    fn display_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let icon = match self.display {
            Display::Monitor(_) => monitor_icon(ui),
            Display::TV(_) => tv_icon(ui),
        }
        .interact(egui::Sense::click());
        icon.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Display menu")
        });
        let entry = (icon | ui.add(egui::Button::new(self.display.short_label()).frame(false)))
            .on_hover_text(format!(
                "Display: {} — click for the display menu",
                self.display.label()
            ));
        egui::Popup::menu(&entry)
            .id(ui.id().with("display_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.display_menu_ui(ui));
    }

    fn cart_status(&self, ui: &mut egui::Ui) {
        let Some(path) = &self.cart_path else { return };
        ui.separator();
        cart_icon(ui).on_hover_text("Cartridge ROM pak");
        ui.label(format!("Cart: {}", file_name(path)));
    }

    /// One entry per port whose source isn't `JoySource::None` — "JR"/"JL"
    /// matching the Joysticks menu's "Right stick"/"Left stick" naming
    /// (`joy.rs`'s `menu_ui`), lit while that port is actively being driven.
    fn joystick_status(&self, ui: &mut egui::Ui) {
        for (stick, prefix, side) in [(RIGHT, "JR", "right"), (LEFT, "JL", "left")] {
            let source = self.joysticks.sources[stick];
            if source == joy::JoySource::None {
                continue;
            }
            ui.separator();
            joystick_icon(ui, self.joysticks.in_use[stick])
                .on_hover_text(format!("Joystick {side} — button/axis active"));
            ui.label(format!("{prefix}: {}", source.label()));
        }
    }

    fn rs232_status(&mut self, ui: &mut egui::Ui) {
        let Some(endpoint) = &self.rs232 else { return };
        // ↑/↓ = bytes out to / in from the host endpoint.
        let (tx, rx) = self
            .machine
            .bus
            .cart
            .as_deluxe_rs232()
            .map_or((0, 0), |pak| (pak.tx_bytes(), pak.rx_bytes()));
        let tx_active = self.activity.rs232_tx.observe(tx);
        let rx_active = self.activity.rs232_rx.observe(rx);
        ui.separator();
        rs232_icon(ui, tx_active || rx_active).on_hover_text("RS-232 — byte sent or received");
        ui.label(format!("RS-232 [{}] ↑{tx} ↓{rx}", endpoint.label()));
    }

    fn mpi_status(&self, ui: &mut egui::Ui) {
        let Some(mpi) = &self.mpi else { return };
        ui.separator();
        mpi_icon(ui).on_hover_text("Multi-Pak Interface");
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
            floppy_icon(ui, active).on_hover_text(format!("Drive {drive} — motor on"));
            ui.label(format!(
                "D{drive}: {}{}",
                file_name(path),
                dirty_mark(dirty)
            ));
        }
    }

    fn vhd_status(&mut self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            let Some(path) = &self.vhd_paths[drive] else {
                continue;
            };
            let count = self.machine.bus.vhd.access_count(drive);
            let active = self.activity.vhd[drive].observe(count);
            ui.separator();
            vhd_icon(ui, active).on_hover_text(format!("VHD drive {drive} — sector I/O"));
            ui.label(format!("VHD{drive}: {}", file_name(path)));
        }
    }

    fn drivewire_status(&mut self, ui: &mut egui::Ui) {
        let Some(dw) = &self.machine.bus.drivewire else {
            return;
        };
        for drive in 0..drivewire::DRIVE_COUNT {
            let Some(path) = &self.dw_paths[drive] else {
                continue;
            };
            let count = dw.drive_ops(drive);
            let active = self.activity.dw[drive].observe(count);
            ui.separator();
            drivewire_icon(ui, active)
                .on_hover_text(format!("DriveWire drive {drive} — sector I/O"));
            ui.label(format!(
                "DW{drive}: {}{}",
                file_name(path),
                dirty_mark(dw.dirty(drive))
            ));
        }
    }

    /// The tape entry — the bar's third entry that is also a control, built
    /// like [`Self::keyboard_status`] and [`Self::display_status`]: the
    /// painted cassette and a frameless-button label form one click target
    /// that pops up the tape menu (`tape_menu_ui` — insert/new/rewind/eject
    /// and the .wav toggle); like the other two, this entry is the only way
    /// in — the Machine menu no longer carries the cassette deck. Unlike
    /// the other two it keeps its readout duties: the icon reddens while
    /// the motor runs (relay closed — CLOAD/CSAVE/MOTOR ON) and its reels
    /// turn with the tape; the label carries the mounted file and the tape
    /// position in bytes (the read head during playback, bytes recorded so
    /// far during CSAVE — [`coco_core::cassette::Cassette::position`]) —
    /// but unlike the disk/DriveWire entries, no dirty mark: recordings now
    /// auto-save themselves (idle auto-finalize, or at rewind —
    /// [`coco_core::cassette::Cassette::finalize_recording`]) rather than
    /// staying dirty until an eject. With nothing mounted the label reads
    /// "No tape" (the deck is always there to click, tape or no tape). See
    /// the keyboard entry's comments for why the popup gets an explicit id
    /// and a `TOP_START` anchor.
    fn tape_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        // The relay is the deck's, so the motor light works with no tape
        // mounted (MOTOR ON) — but the reels are the cassette's, so they
        // only turn while one is in.
        let motor = self.machine.bus.pia1.a.c2_output();
        let (pos, len) = self.machine.bus.cassette.position();
        let dt = ui.input(|i| i.stable_dt);
        let angle = self
            .activity
            .tape_reel
            .advance(pos, motor && self.tape_path.is_some(), dt);
        let icon = cassette_icon(ui, motor, angle).interact(egui::Sense::click());
        icon.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Tape menu")
        });
        let label = match &self.tape_path {
            Some(path) => format!("{} [{pos}/{len}]", file_name(path)),
            None => "No tape".to_string(),
        };
        let entry = (icon | ui.add(egui::Button::new(label).frame(false))).on_hover_text(format!(
            "Cassette deck: {} — click for the tape menu",
            self.tape_path.as_deref().map_or("no tape", file_name)
        ));
        // Not the menu-default `CloseOnClick`: that closes on clicks *inside*
        // the popup too, which kills the seek field the moment it's clicked
        // for typing. Every action item in `tape_menu_ui` calls `ui.close()`
        // itself, so items still dismiss the menu; only the seek field and
        // the .wav checkbox keep it open.
        egui::Popup::menu(&entry)
            .id(ui.id().with("tape_menu"))
            .align(egui::RectAlign::TOP_START)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.tape_menu_ui(ui));
    }

    /// Shown whenever a printer sink is plugged into the bit-banger:
    /// text-file capture ([`Self::print_capture_path`]) or the paper
    /// window's live DMP-105 (`Self::paper_window`'s `handle`, kept in
    /// lockstep with the bit-banger's sink on every attach/detach/restore
    /// path).
    /// A capture path wins the label if somehow both are true at once
    /// (shouldn't happen — starting either kind of capture detaches the
    /// other — but the label has to pick one).
    fn printer_status(&mut self, ui: &mut egui::Ui) {
        let bitbanger = &self.machine.bus.bitbanger;
        let capture_path = self.print_capture_path.as_deref();
        if capture_path.is_none() && self.paper_window.handle.is_none() {
            return;
        }
        let active = self.activity.printer.observe(bitbanger.bytes_out());
        ui.separator();
        printer_icon(ui, active).on_hover_text("Printer — byte received");
        ui.label(format!(
            "Printer: {}",
            capture_path.map_or("DMP-105", file_name)
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
        MPISlot::GamesMasterCartridge(p) => format!("GMC:{}", file_name(p)),
        MPISlot::Orch90(p) => format!("Orchestra-90:{}", file_name(p)),
        MPISlot::SoundSpeechCartridge => "SSC".to_string(),
    }
}
