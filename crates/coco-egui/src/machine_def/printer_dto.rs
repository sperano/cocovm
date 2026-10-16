//! `[drivewire.printer]`: where finished DriveWire print jobs go
//! (`coco_core::drivewire::printer`).
//!
//! ```toml
//! [drivewire.printer]
//! output = "folder"   # "off" (default) | "folder" | "printer-port"
//! folder = "prints"   # host folder; relative paths resolve like `[[drivewire.shares]]`
//! format = "text"     # "text" (default) | "raw"
//! ```
//!
//! `output` defaults to `"off"`, so enabling DriveWire never starts printing
//! anywhere the user didn't choose — the same rationale as
//! `coco_core::drivewire::PrintOutput::Off`'s doc. `"printer-port"` needs a
//! sink wired to the built-in serial port (`[ports].serial`); without one
//! [`DriveWirePrinterDTO::validate`] rejects it.

use coco_core::drivewire::{PrintFormat, PrintOutput};
use serde::{Deserialize, Serialize};

use super::SerialDTO;

/// Why a Folder output needs a folder.
const NO_FOLDER_ERROR: &str = "Choose a folder for DriveWire print jobs";
/// Why a Printer port output needs something attached under Ports.
const NO_PRINTER_SINK_ERROR: &str = "DriveWire print jobs are set to the printer port, but \
    nothing is attached there; pick a printer or capture file under Ports";

/// `output` key values. Maps to [`PrintOutput`], minus the folder/format
/// payload the `folder`/`format` keys carry instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrintOutputDTO {
    #[default]
    Off,
    Folder,
    PrinterPort,
}

/// `format` key values. Maps to [`PrintFormat`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrintFormatDTO {
    #[default]
    Text,
    Raw,
}

impl From<PrintFormatDTO> for PrintFormat {
    fn from(format: PrintFormatDTO) -> Self {
        match format {
            PrintFormatDTO::Text => PrintFormat::Text,
            PrintFormatDTO::Raw => PrintFormat::Raw,
        }
    }
}

/// `[drivewire.printer]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveWirePrinterDTO {
    #[serde(default)]
    pub output: PrintOutputDTO,
    /// Host folder for [`PrintOutputDTO::Folder`]; ignored otherwise. Relative paths resolve
    /// against the machine's artifact directory, like `[[drivewire.shares]]`.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub format: PrintFormatDTO,
}

impl DriveWirePrinterDTO {
    /// Whether this section is the all-default `Off` one, so [`super::DriveWireDTO`] can skip
    /// writing it and an existing definition round-trips unchanged.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The runtime [`PrintOutput`] this section asks for, with `folder` resolved against
    /// `slug`'s artifact directory.
    pub fn output(&self, slug: &str) -> PrintOutput {
        match self.output {
            PrintOutputDTO::Off => PrintOutput::Off,
            PrintOutputDTO::PrinterPort => PrintOutput::PrinterPort,
            PrintOutputDTO::Folder => PrintOutput::Folder {
                dir: super::resolve_media_path(&self.folder, slug),
                format: self.format.into(),
            },
        }
    }

    /// Checks `output` against what it needs: a folder, or (for the printer port) a `serial`
    /// sink already attached.
    pub fn validate(&self, serial: Option<SerialDTO>) -> Result<(), String> {
        match self.output {
            PrintOutputDTO::Folder if self.folder.trim().is_empty() => {
                Err(NO_FOLDER_ERROR.to_string())
            }
            PrintOutputDTO::PrinterPort if serial.is_none() => {
                Err(NO_PRINTER_SINK_ERROR.to_string())
            }
            PrintOutputDTO::Off | PrintOutputDTO::Folder | PrintOutputDTO::PrinterPort => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "printer_dto_test.rs"]
mod tests;
