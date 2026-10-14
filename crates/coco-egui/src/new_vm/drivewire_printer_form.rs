//! The DriveWire tab's "Printer" rows: where finished print jobs go
//! (`[drivewire.printer]`, `coco_core::drivewire::printer`), and a running
//! VM's print diagnostics.

use std::path::Path;

use coco_core::drivewire::{PRINT_JOB_PART_BYTES, PrintDiagnostics};
use eframe::egui;

use super::MachineForm;
use super::drivewire_shares_form::folder_dialog;
use crate::chrome::status_bar::file_name;
use crate::machine_def::{self, DriveWirePrinterDTO, PrintFormatDTO, PrintOutputDTO, SerialDTO};

const OUTPUT_COMBO_WIDTH: f32 = 130.0;
const FORMAT_COMBO_WIDTH: f32 = 170.0;
const BROWSE_BUTTON_WIDTH: f32 = 76.0;
const MIN_FOLDER_FIELD_WIDTH: f32 = 80.0;
const ROW_GAPS: f32 = 4.0;
const OUTPUT_ROW_LABEL: &str = "Print jobs to";
/// The output combo's accessible name — distinct from [`OUTPUT_ROW_LABEL`]'s visible text so a
/// query for either finds exactly one node (the named-objects row's `ROW_LABEL`/`COMBO_LABEL`
/// pair in `drivewire_named_form.rs` does the same).
const OUTPUT_COMBO_LABEL: &str = "DriveWire print output";
const OFF_LABEL: &str = "Nowhere (off)";
const FOLDER_LABEL: &str = "Folder";
const PRINTER_PORT_LABEL: &str = "Printer port";
const FORMAT_ROW_LABEL: &str = "Format";
/// The format combo's accessible name; see [`OUTPUT_COMBO_LABEL`].
const FORMAT_COMBO_LABEL: &str = "Printer output format";
const TEXT_FORMAT_LABEL: &str = "Text (LF line ends)";
const RAW_FORMAT_LABEL: &str = "Raw bytes";
const EMPTY_FOLDER_HINT: &str = "No folder";
const PRINTER_PORT_NOTE: &str = "Jobs print on the printer or capture file attached under Ports.";
const BYTES_PER_MIB: usize = 1024 * 1024;

impl MachineForm {
    /// The output picker, the folder/format or printer-port note it implies, any validation
    /// problem, a running VM's status lines, and a closing hint.
    pub(crate) fn drivewire_printer_rows(
        &mut self,
        ui: &mut egui::Ui,
        slug: &str,
        diagnostics: Option<&PrintDiagnostics>,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = super::FORM_GRID_SPACING[1];
            ui.add_enabled_ui(self.drivewire.enabled, |ui| {
                ui.push_id((self.salt, "drivewire_printer"), |ui| {
                    output_row(ui, &mut self.drivewire.printer);
                    match self.drivewire.printer.output {
                        PrintOutputDTO::Folder => {
                            folder_rows(ui, slug, &mut self.drivewire.printer);
                        }
                        PrintOutputDTO::PrinterPort => {
                            ui.small(PRINTER_PORT_NOTE);
                        }
                        PrintOutputDTO::Off => {}
                    }
                });
            });
            // `validate_drivewire` checks the printer only while DriveWire is enabled.
            let serial = Option::<SerialDTO>::from(self.serial);
            if self.drivewire.enabled
                && let Err(problem) = self.drivewire.printer.validate(serial)
            {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
            }
            if self.drivewire.enabled
                && let Some(diagnostics) = diagnostics
            {
                status_rows(ui, diagnostics);
            }
            ui.small(printer_hint());
        });
    }
}

fn output_row(ui: &mut egui::Ui, printer: &mut DriveWirePrinterDTO) {
    ui.horizontal(|ui| {
        ui.label(OUTPUT_ROW_LABEL);
        let combo = egui::ComboBox::from_id_salt("drivewire_printer_output")
            .width(OUTPUT_COMBO_WIDTH)
            .selected_text(output_label(printer.output))
            .show_ui(ui, |ui| {
                for choice in [
                    PrintOutputDTO::Off,
                    PrintOutputDTO::Folder,
                    PrintOutputDTO::PrinterPort,
                ] {
                    ui.selectable_value(&mut printer.output, choice, output_label(choice));
                }
            });
        combo.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                ui.is_enabled(),
                OUTPUT_COMBO_LABEL,
            )
        });
    });
}

fn output_label(output: PrintOutputDTO) -> &'static str {
    match output {
        PrintOutputDTO::Off => OFF_LABEL,
        PrintOutputDTO::Folder => FOLDER_LABEL,
        PrintOutputDTO::PrinterPort => PRINTER_PORT_LABEL,
    }
}

/// The folder field, Browse, a resolved-path preview for a relative folder, and the Format
/// combo — shown only while [`PrintOutputDTO::Folder`] is picked.
fn folder_rows(ui: &mut egui::Ui, slug: &str, printer: &mut DriveWirePrinterDTO) {
    let height = ui.spacing().interact_size.y;
    ui.horizontal(|ui| {
        let folder_width =
            (ui.available_width() - BROWSE_BUTTON_WIDTH - ROW_GAPS * ui.spacing().item_spacing.x)
                .max(MIN_FOLDER_FIELD_WIDTH);
        let field = ui.add_sized(
            [folder_width, height],
            egui::TextEdit::singleline(&mut printer.folder)
                .id_salt("folder")
                .hint_text(EMPTY_FOLDER_HINT),
        );
        field.widget_info(|| {
            let mut info = egui::WidgetInfo::text_edit(
                ui.is_enabled(),
                &printer.folder,
                &printer.folder,
                EMPTY_FOLDER_HINT,
            );
            info.label = Some("Printer output folder".to_string());
            info
        });
        let browse = ui.add_sized([BROWSE_BUTTON_WIDTH, height], egui::Button::new("Browse…"));
        browse.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                "Browse printer output folder…",
            )
        });
        if browse.clicked()
            && let Some(selected) = folder_dialog(&printer.folder).pick_folder()
        {
            printer.folder = selected.to_string_lossy().into_owned();
        }
    });
    if !printer.folder.trim().is_empty() && !Path::new(&printer.folder).is_absolute() {
        ui.small(format!(
            "Resolves to {}",
            machine_def::resolve_media_path(&printer.folder, slug).display()
        ));
    }
    format_row(ui, &mut printer.format);
}

fn format_row(ui: &mut egui::Ui, format: &mut PrintFormatDTO) {
    ui.horizontal(|ui| {
        ui.label(FORMAT_ROW_LABEL);
        let combo = egui::ComboBox::from_id_salt("drivewire_printer_format")
            .width(FORMAT_COMBO_WIDTH)
            .selected_text(format_label(*format))
            .show_ui(ui, |ui| {
                for choice in [PrintFormatDTO::Text, PrintFormatDTO::Raw] {
                    ui.selectable_value(format, choice, format_label(choice));
                }
            });
        combo.response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                ui.is_enabled(),
                FORMAT_COMBO_LABEL,
            )
        });
    });
}

fn format_label(format: PrintFormatDTO) -> &'static str {
    match format {
        PrintFormatDTO::Text => TEXT_FORMAT_LABEL,
        PrintFormatDTO::Raw => RAW_FORMAT_LABEL,
    }
}

fn status_rows(ui: &mut egui::Ui, diagnostics: &PrintDiagnostics) {
    for line in status_lines(diagnostics) {
        ui.label(line);
    }
    if let Some(error) = &diagnostics.last_error {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("Last printer error: {error}"),
        );
    }
}

/// One line per counter worth showing: printed (and the last file written),
/// waiting, the open job's size, then discarded/dropped/incomplete only
/// when nonzero. Factored out of [`status_rows`] so it's testable without a
/// `egui::Ui`.
fn status_lines(diagnostics: &PrintDiagnostics) -> Vec<String> {
    let mut lines = vec![printed_line(diagnostics)];
    if diagnostics.waiting > 0 {
        lines.push(format!(
            "{} job{} waiting to be written",
            diagnostics.waiting,
            plural_s(diagnostics.waiting as u64)
        ));
    }
    if diagnostics.open_bytes > 0 {
        lines.push(format!("Open job: {} bytes", diagnostics.open_bytes));
    }
    for (count, reason) in [
        (diagnostics.discarded, "discarded (printing was off)"),
        (diagnostics.dropped, "dropped (the queue was full)"),
        (
            diagnostics.incomplete,
            "incomplete (ended by a reset or stop)",
        ),
    ] {
        if count > 0 {
            lines.push(format!("{count} job{} {reason}", plural_s(count)));
        }
    }
    lines
}

fn printed_line(diagnostics: &PrintDiagnostics) -> String {
    let base = format!(
        "Printed {} job{}",
        diagnostics.printed,
        plural_s(diagnostics.printed)
    );
    match &diagnostics.last_file {
        Some(path) => format!("{base}, last {}", file_name(path)),
        None => base,
    }
}

fn plural_s(count: u64) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// [`PRINT_JOB_PART_BYTES`] spelled out in MiB, so the hint never drifts
/// from the real limit. The core counts each part of a large job as one
/// job (`PrintDiagnostics::printed`), and the hint says so.
fn printer_hint() -> String {
    format!(
        "NitrOS-9 prints to /P over DriveWire. Each closed /P path is one job; jobs larger \
         than {} MiB are written in parts, and each part counts as one job.",
        PRINT_JOB_PART_BYTES / BYTES_PER_MIB
    )
}

#[cfg(test)]
#[path = "drivewire_printer_form_test.rs"]
mod tests;
