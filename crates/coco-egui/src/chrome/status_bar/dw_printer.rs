//! The status bar's DriveWire hover text for finished print jobs
//! (`coco_core::drivewire::printer`) — split out of `status_bar.rs`
//! (`super::drivewire_hover`) to keep that file under the project's
//! ~500-line ceiling.

use coco_core::drivewire::{DWServer, PrintOutput};

/// Extra hover text appended after [`super::drivewire_hover`]'s host-service lines.
/// Empty while output is off and nothing has ever printed, discarded, or dropped a
/// job — an unused printer adds no noise to the DriveWire hover. Returning `String`
/// rather than `Option<String>` lets the call site stay a single `push_str`.
pub(super) fn hover_lines(dw: &DWServer) -> String {
    let diagnostics = dw.print_diagnostics();
    let ever_active = !matches!(dw.print_output(), PrintOutput::Off)
        || diagnostics.printed > 0
        || diagnostics.waiting > 0
        || diagnostics.open_bytes > 0
        || diagnostics.discarded > 0
        || diagnostics.dropped > 0
        || diagnostics.incomplete > 0;
    if !ever_active {
        return String::new();
    }
    let mut hover = format!(
        "\nPrinter: {} printed, {} waiting",
        diagnostics.printed, diagnostics.waiting
    );
    if let Some(error) = &diagnostics.last_error {
        hover.push_str(&format!("\nPrinter error: {error}"));
    }
    hover
}

#[cfg(test)]
#[path = "dw_printer_test.rs"]
mod tests;
