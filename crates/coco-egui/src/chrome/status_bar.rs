use coco_core::joystick::{LEFT, RIGHT};

use crate::*;

/// Status-bar marker text of a suspended machine.
const SUSPENDED_STATUS: &str = "Suspended";
/// Hover text of that marker: what the window can still do.
const SUSPENDED_STATUS_HOVER: &str =
    "Frozen to disk — press Start to resume, or close the window to keep it suspended";
/// Hover text of the sound entry.
const SOUND_HOVER: &str = "Sound output — click for mute and volume controls";
/// [`SOUND_HOVER`] while an Orchestra-90 is inserted: its levels toggle is in the same menu.
const SOUND_ORCH90_HOVER: &str =
    "Sound output — click for mute and volume controls and the Orchestra-90 levels";
/// Readout of an installed FD-502 with no disk in any drive.
pub(crate) const NO_DISKS_READOUT: &str = "No disks";
/// Hover text of that readout: where to mount one.
pub(crate) const NO_DISKS_HOVER: &str = "FD-502 — no disk mounted; click for the disks menu";

impl CocoApp {
    /// The status bar: live readouts, plus the seven entries that double as control menus.
    /// Height is pinned to [`STATUS_BAR_H`] to match what the window-sizing math reserves for it.
    /// While suspended the readouts draw disabled (no popups) under a "Suspended" marker.
    /// Under `status_bar_icons_only` (`config.rs`) each iconed entry drops its readout into
    /// hover text ([`readout`], [`menu_entry`]); the icon-less Suspended marker, toast and
    /// runtime readout are unaffected.
    pub(crate) fn status_bar_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(STATUS_BAR_H)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.suspended {
                        self.suspended_status(ui);
                        ui.disable();
                    }
                    self.keyboard_status(ui);
                    self.display_status(ui);
                    self.sound_status(ui);
                    self.tape_status(ui);
                    self.joystick_status(ui);
                    self.cart_status(ui);
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
                    // Last unless a toast is showing (bumps it one slot right).
                    self.runtime_status(ui);
                });
            });
    }

    /// Leading "Suspended" marker of a frozen machine's status bar.
    fn suspended_status(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(SUSPENDED_STATUS).strong())
            .on_hover_text(SUSPENDED_STATUS_HOVER);
        ui.separator();
    }

    /// The keyboard entry: icon + mode label are one click target that opens the
    /// keyboard menu — the only way in, since the window has no menu bar.
    fn keyboard_status(&mut self, ui: &mut egui::Ui) {
        let icon = keyboard_icon(ui).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Keyboard menu");
        let mode = self.kb_mode.label();
        let hotkey = ui
            .ctx()
            .format_shortcut(&self.hotkeys.keyboard_mode.shortcut());
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, mode).on_hover_text(format!(
            "Keyboard input mode: {mode} — click for the keyboard menu ({hotkey} toggles)"
        ));
        egui::Popup::menu(&entry)
            // Explicit id: the icon's auto id shifts if an earlier entry becomes conditional.
            .id(ui.id().with("keyboard_menu"))
            // TOP_START: egui's own flip-to-fit only kicks in after the first (zero-height) frame.
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.keyboard_menu_ui(ui));
    }

    /// The display entry, built like [`Self::keyboard_status`]: icon + label open the
    /// display menu — the only way to switch [`Display`] modes.
    fn display_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let icon = match self.display {
            Display::Monitor(_) => monitor_icon(ui),
            Display::TV(_) => tv_icon(ui),
        }
        .interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Display menu");
        let entry = menu_entry(
            ui,
            self.status_bar_icons_only,
            icon,
            self.display.short_label(),
        )
        .on_hover_text(format!(
            "Display: {} — click for the display menu",
            self.display.label()
        ));
        egui::Popup::menu(&entry)
            .id(ui.id().with("display_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.display_menu_ui(ui));
    }

    /// The sound entry: a speaker icon and optional label open the Sound menu (mute, volume,
    /// and the Orchestra-90 levels toggle while one is inserted).
    fn sound_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let icon = speaker_icon(ui).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Sound menu");
        let hover = if self.machine.bus.cart.as_orch90().is_some() {
            SOUND_ORCH90_HOVER
        } else {
            SOUND_HOVER
        };
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, "Sound").on_hover_text(hover);
        egui::Popup::menu(&entry)
            .id(ui.id().with("sound_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.sound_menu_ui(ui));
    }

    fn cart_status(&self, ui: &mut egui::Ui) {
        let Some(path) = &self.cart_path else { return };
        ui.separator();
        let label = format!("Cart: {}", file_name(path));
        let icon = cart_icon(ui);
        readout(ui, self.status_bar_icons_only, icon, label).on_hover_text("Cartridge ROM pak");
    }

    /// The joysticks entry, built like [`Self::keyboard_status`]: icon lights while either
    /// port is active; label lists assigned sources (for example, "R: Keys ·
    /// L: Mouse") or "No joysticks".
    fn joystick_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let either_active = self.joysticks.in_use[RIGHT] || self.joysticks.in_use[LEFT];
        let icon = joystick_icon(ui, either_active).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Joysticks menu");
        let assigned: Vec<String> = [(RIGHT, "R"), (LEFT, "L")]
            .into_iter()
            .filter_map(|(stick, prefix)| {
                let source = self.joysticks.sources[stick];
                (source != joy::JoySource::None).then(|| format!("{prefix}: {}", source.label()))
            })
            .collect();
        let label = if assigned.is_empty() {
            "No joysticks".to_string()
        } else {
            assigned.join(" · ")
        };
        // Icon merges both ports; hover gives the per-port detail ("off", not the raw variant
        // name).
        let port_hover = |stick: usize| {
            let source = self.joysticks.sources[stick];
            if source == joy::JoySource::None {
                "off".to_string()
            } else if self.joysticks.in_use[stick] {
                format!("{} (active)", source.label())
            } else {
                source.label().to_string()
            }
        };
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, label).on_hover_text(format!(
            "Joysticks — right: {}, left: {} — click for the joysticks menu",
            port_hover(RIGHT),
            port_hover(LEFT)
        ));
        egui::Popup::menu(&entry)
            .id(ui.id().with("joysticks_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.joysticks.menu_ui(ui));
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
        let label = format!("RS-232 [{}] ↑{tx} ↓{rx}", endpoint.label());
        let icon = rs232_icon(ui, tx_active || rx_active);
        readout(ui, self.status_bar_icons_only, icon, label)
            .on_hover_text("RS-232 — byte sent or received");
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
        let label = format!("MPI [{}]", slots.join(" "));
        let icon = mpi_icon(ui);
        readout(ui, self.status_bar_icons_only, icon, label).on_hover_text("Multi-Pak Interface");
    }

    /// The FD-502's drives, built like [`Self::tape_status`]: one entry per mounted disk,
    /// or a single "No disks" entry with nothing mounted. Every entry opens the same
    /// all-drives menu (the clicked drive's section first), so an empty drive stays
    /// mountable beside a full one. No controller, no entry — the FD-502 is a launch-time
    /// peripheral.
    fn disk_status(&mut self, ui: &mut egui::Ui) {
        let Some(disk_cart) = self.machine.bus.cart.as_disk_cart() else {
            return;
        };
        // Read the lights up front: the popups below need `&mut self`.
        let motor_on: [bool; UI_DRIVES] =
            std::array::from_fn(|drive| disk_cart.drive_active(drive));
        // "*" = modified in memory; written back on eject/exit.
        let dirty: [bool; UI_DRIVES] =
            std::array::from_fn(|drive| disk_cart.disk(drive).is_some_and(|d| d.dirty()));
        if self.disk_paths.iter().all(Option::is_none) {
            // Motor light works with no disk mounted (DIR on an empty drive spins it).
            self.no_disks_entry(ui, motor_on.iter().any(|&on| on));
            return;
        }
        for drive in 0..UI_DRIVES {
            self.drive_entry(ui, drive, motor_on[drive], dirty[drive]);
        }
    }

    /// The "No disks" placeholder of [`Self::disk_status`].
    fn no_disks_entry(&mut self, ui: &mut egui::Ui, motor_on: bool) {
        ui.separator();
        let icon = floppy_icon(ui, motor_on).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Disks menu");
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, NO_DISKS_READOUT)
            .on_hover_text(NO_DISKS_HOVER);
        egui::Popup::menu(&entry)
            .id(ui.id().with("disks_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.disks_menu_ui(ui, 0));
    }

    /// One drive's entry of [`Self::disk_status`]; nothing when the drive is empty.
    fn drive_entry(&mut self, ui: &mut egui::Ui, drive: usize, motor_on: bool, dirty: bool) {
        let Some(path) = &self.disk_paths[drive] else {
            return;
        };
        ui.separator();
        let label = format!("D{drive}: {}{}", file_name(path), dirty_mark(dirty));
        let icon = floppy_icon(ui, motor_on).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, format!("Drive {drive} menu"));
        let motor = if motor_on { "on" } else { "off" };
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, label).on_hover_text(format!(
            "Drive {drive} — motor {motor} — click for the disks menu"
        ));
        egui::Popup::menu(&entry)
            .id(ui.id().with(("drive_menu", drive)))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.disks_menu_ui(ui, drive));
    }

    fn vhd_status(&mut self, ui: &mut egui::Ui) {
        for drive in 0..UI_DRIVES {
            let Some(path) = &self.vhd_paths[drive] else {
                continue;
            };
            let count = self.machine.bus.vhd.access_count(drive);
            let active = self.activity.vhd[drive].observe(count);
            ui.separator();
            let label = format!("VHD{drive}: {}", file_name(path));
            let icon = vhd_icon(ui, active);
            readout(ui, self.status_bar_icons_only, icon, label)
                .on_hover_text(format!("VHD drive {drive} — sector I/O"));
        }
    }

    fn drivewire_status(&mut self, ui: &mut egui::Ui) {
        let Some(dw) = &self.machine.bus.drivewire else {
            return;
        };
        if self.dw_paths.iter().all(Option::is_none) {
            ui.separator();
            let icon = drivewire_icon(ui, false);
            readout(ui, self.status_bar_icons_only, icon, "DriveWire")
                .on_hover_text(drivewire_hover(dw, None));
            return;
        }
        for drive in 0..drivewire::DRIVE_COUNT {
            let Some(path) = &self.dw_paths[drive] else {
                continue;
            };
            let count = dw.drive_ops(drive);
            let active = self.activity.dw[drive].observe(count);
            ui.separator();
            let label = format!(
                "DW{drive}: {}{}",
                file_name(path),
                dirty_mark(dw.dirty(drive))
            );
            let icon = drivewire_icon(ui, active);
            readout(ui, self.status_bar_icons_only, icon, label)
                .on_hover_text(drivewire_hover(dw, Some(drive)));
        }
    }

    /// The tape entry, built like [`Self::keyboard_status`]: cassette icon reddens while
    /// the motor runs; label shows the mounted file and tape position, or "No tape".
    fn tape_status(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        // Motor light works with no tape mounted; reels only turn once one is in.
        let motor = self.machine.bus.pia1.a.c2_output();
        let (pos, len) = self.machine.bus.cassette.position();
        let dt = ui.input(|i| i.stable_dt);
        let angle = self
            .activity
            .tape_reel
            .advance(pos, motor && self.tape_path.is_some(), dt);
        let icon = cassette_icon(ui, motor, angle).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Tape menu");
        let label = match &self.tape_path {
            Some(path) => format!("{} [{pos}/{len}]", file_name(path)),
            None => "No tape".to_string(),
        };
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, label).on_hover_text(format!(
            "Cassette deck: {} — click for the tape menu",
            self.tape_path.as_deref().map_or("no tape", file_name)
        ));
        // CloseOnClickOutside, not the default: the default would close the popup on a
        // seek-field click too.
        egui::Popup::menu(&entry)
            .id(ui.id().with("tape_menu"))
            .align(egui::RectAlign::TOP_START)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.tape_menu_ui(ui));
    }

    /// Always shown: the icon flashes on serial-port (bit-banger) output even with no
    /// sink attached. Click the icon for a menu to toggle the printer paper window or
    /// start, stop, and open a print capture. The label names the attached sink.
    fn printer_status(&mut self, ui: &mut egui::Ui) {
        let bytes_out = self.machine.bus.bitbanger.bytes_out();
        let active = self.activity.printer.observe(bytes_out);
        ui.separator();
        let icon = printer_icon(ui, active).interact(egui::Sense::click());
        name_menu_icon(ui, &icon, "Printer menu");
        let capture_path = self.print_capture_path.as_deref();
        let sink_label = capture_path.map(file_name).or_else(|| {
            self.paper_window
                .handle
                .as_ref()
                .map(|handle| match handle.model() {
                    coco_core::dmp::DmpModel::Dmp105 => "DMP-105",
                    coco_core::dmp::DmpModel::Dmp130 => "DMP-130",
                })
        });
        let label_text = match sink_label {
            Some(name) => format!("Printer: {name}"),
            None => "Printer".to_string(),
        };
        let entry = menu_entry(ui, self.status_bar_icons_only, icon, label_text)
            .on_hover_text("Printer — byte sent on the serial port — click for menu");
        egui::Popup::menu(&entry)
            .id(ui.id().with("printer_menu"))
            .align(egui::RectAlign::TOP_START)
            .show(|ui| self.printer_menu_ui(ui));
    }

    /// The bar's last entry (except while a toast is showing): cumulative powered-on time.
    /// A plain label, not a click target — nothing to toggle, so no icon.
    fn runtime_status(&self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(format!(
            "Runtime: {}",
            humanize_runtime(self.total_runtime.as_secs())
        ))
        .on_hover_text("Total powered-on time, all sessions");
    }
}

fn drivewire_hover(dw: &coco_core::drivewire::DWServer, drive: Option<usize>) -> String {
    use crate::media::drivewire::guest_media_note;
    use coco_core::drivewire::host::HostError;
    let diagnostics = dw.host_diagnostics();
    let heading = drive.map_or_else(
        || "DriveWire host services".to_string(),
        |drive| {
            format!(
                "DriveWire drive {drive} — sector I/O{}",
                guest_media_note(dw, drive)
            )
        },
    );
    let mut hover = format!(
        "{heading}\nHost: {:?}, pending: {}, outstanding: {}\nCompleted: {}, errors: {}, cancelled: {}, stale: {}, backpressure: {}",
        diagnostics.state,
        diagnostics.pending,
        diagnostics.outstanding,
        diagnostics.completions,
        diagnostics.errors,
        diagnostics.cancelled,
        diagnostics.stale,
        diagnostics.backpressure,
    );
    match diagnostics.last_error {
        Some(HostError::Share(error)) => hover.push_str(&format!("\nLast error: share {error}")),
        Some(error) => hover.push_str(&format!("\nLast error: {error:?}")),
        None => {}
    }
    let channels = dw.channel_diagnostics();
    hover.push_str(&format!(
        "\nVirtual channels: {} open, {} bytes to guest, {} from guest\nDropped: {}, unknown channel: {}, short reads: {}",
        channels.open,
        channels.to_guest,
        channels.from_guest,
        channels.dropped_bytes,
        channels.unknown_channel_ops,
        channels.short_reads,
    ));
    hover
}

/// Names `icon` as the button that opens `menu` — the bare icon has no accessible name of
/// its own (`status_icons::paint`).
fn name_menu_icon(ui: &egui::Ui, icon: &egui::Response, menu: impl Into<String>) {
    let menu = menu.into();
    icon.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), menu.clone())
    });
}

/// A passive entry's readout after its `icon`: drawn as a label, or under `icons_only`
/// folded into `icon`'s hover text and accessible name instead. Returns `icon` so the
/// caller's own hover text stacks after the readout, like `widgets::toolbar_button`.
fn readout(
    ui: &mut egui::Ui,
    icons_only: bool,
    icon: egui::Response,
    text: impl Into<String>,
) -> egui::Response {
    if icons_only {
        let text = text.into();
        // The bare icon has no accessible name of its own (`status_icons::paint`).
        icon.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text.clone())
        });
        return hover_text_even_disabled(icon, text);
    }
    ui.label(text.into());
    icon
}

/// A menu entry's readout after its `icon`: a frameless button unioned with `icon` so
/// both halves open the menu, or under `icons_only` a hover text on `icon` alone — which
/// stays the click target, keeping its own accessible name (the menu, not the text).
fn menu_entry(
    ui: &mut egui::Ui,
    icons_only: bool,
    icon: egui::Response,
    text: impl Into<String>,
) -> egui::Response {
    if icons_only {
        return hover_text_even_disabled(icon, text.into());
    }
    icon | ui.add(egui::Button::new(text.into()).frame(false))
}

/// `text` as `response`'s tooltip in both the enabled and the disabled state: a
/// suspended machine's bar is disabled (`status_bar_ui`), and in icons-only mode the
/// tooltip is the readout's only remaining carrier.
fn hover_text_even_disabled(response: egui::Response, text: String) -> egui::Response {
    response
        .on_hover_text(text.clone())
        .on_disabled_hover_text(text)
}

/// The file name of a mounted image, for a one-line readout or menu label.
pub(crate) fn file_name(path: &Path) -> &str {
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
        MPISlot::BankedROMPak(p) => format!("Banked:{}", file_name(p)),
        MPISlot::FD502 => "FD-502".to_string(),
        MPISlot::DistoRTC => "RTC".to_string(),
        MPISlot::DeluxeRS232(_) => "RS-232".to_string(),
        MPISlot::GamesMasterCartridge(p) => format!("GMC:{}", file_name(p)),
        MPISlot::Orch90 => "Orchestra-90".to_string(),
        MPISlot::SoundSpeechCartridge => "SSC".to_string(),
        MPISlot::CoCoMax => "CoCo Max".to_string(),
    }
}
