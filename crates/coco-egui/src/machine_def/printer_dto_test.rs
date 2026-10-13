use std::path::PathBuf;

use coco_core::drivewire::{PrintFormat, PrintOutput};

use super::*;
use crate::machine_def::{self, MachineDef, SerialDTO};

const PRINTER_TOML: &str = r#"
schema = 1
name = "Printer"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"

[drivewire]
enabled = true

[drivewire.printer]
output = "folder"
folder = "prints"
format = "raw"
"#;

const NO_PRINTER_TOML: &str = r#"
schema = 1
name = "No printer"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"

[drivewire]
enabled = true
"#;

fn parsed() -> MachineDef {
    toml::from_str(PRINTER_TOML).expect("printer section parses")
}

#[test]
fn a_definition_without_the_section_parses_to_off() {
    let def: MachineDef = toml::from_str(NO_PRINTER_TOML).expect("legacy definition parses");
    assert_eq!(def.drivewire.printer, DriveWirePrinterDTO::default());
    assert_eq!(def.drivewire.printer.output("slug"), PrintOutput::Off);
}

#[test]
fn the_default_section_is_omitted_on_save_and_round_trips() {
    let def = MachineDef::from_config(
        "Default".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    let written = toml::to_string(&def).unwrap();
    assert!(!written.contains("[drivewire.printer]"), "{written}");
    assert_eq!(toml::from_str::<MachineDef>(&written).unwrap(), def);
}

#[test]
fn a_configured_section_round_trips_through_toml() {
    let def = parsed();
    let written = toml::to_string(&def).unwrap();
    assert!(written.contains("[drivewire.printer]"), "{written}");
    assert!(written.contains("output = \"folder\""), "{written}");
    assert!(written.contains("format = \"raw\""), "{written}");
    assert_eq!(toml::from_str::<MachineDef>(&written).unwrap(), def);
}

#[test]
fn each_output_value_parses() {
    assert_eq!(output_from_toml("off"), PrintOutputDTO::Off);
    assert_eq!(output_from_toml("folder"), PrintOutputDTO::Folder);
    assert_eq!(
        output_from_toml("printer-port"),
        PrintOutputDTO::PrinterPort
    );
}

#[test]
fn each_format_value_parses() {
    assert_eq!(format_from_toml("text"), PrintFormatDTO::Text);
    assert_eq!(format_from_toml("raw"), PrintFormatDTO::Raw);
}

fn output_from_toml(value: &str) -> PrintOutputDTO {
    #[derive(serde::Deserialize)]
    struct Wrap {
        output: PrintOutputDTO,
    }
    toml::from_str::<Wrap>(&format!("output = \"{value}\""))
        .unwrap()
        .output
}

fn format_from_toml(value: &str) -> PrintFormatDTO {
    #[derive(serde::Deserialize)]
    struct Wrap {
        format: PrintFormatDTO,
    }
    toml::from_str::<Wrap>(&format!("format = \"{value}\""))
        .unwrap()
        .format
}

#[test]
fn off_resolves_directly_regardless_of_folder_or_format() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::Off,
        folder: "ignored".into(),
        format: PrintFormatDTO::Raw,
    };
    assert_eq!(printer.output("slug"), PrintOutput::Off);
}

#[test]
fn printer_port_resolves_directly() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::PrinterPort,
        ..DriveWirePrinterDTO::default()
    };
    assert_eq!(printer.output("slug"), PrintOutput::PrinterPort);
}

#[test]
fn folder_output_resolves_a_relative_path_against_the_slug() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::Folder,
        folder: "prints".into(),
        format: PrintFormatDTO::Raw,
    };
    assert_eq!(
        printer.output("my-slug"),
        PrintOutput::Folder {
            dir: machine_def::resolve_media_path("prints", "my-slug"),
            format: PrintFormat::Raw,
        }
    );
}

#[test]
fn folder_output_keeps_an_absolute_path_as_is() {
    let absolute = if cfg!(windows) {
        "C:\\prints"
    } else {
        "/srv/prints"
    };
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::Folder,
        folder: absolute.into(),
        format: PrintFormatDTO::Text,
    };
    assert_eq!(
        printer.output("my-slug"),
        PrintOutput::Folder {
            dir: PathBuf::from(absolute),
            format: PrintFormat::Text,
        }
    );
}

#[test]
fn folder_output_without_a_folder_fails_validation() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::Folder,
        folder: "   ".into(),
        ..DriveWirePrinterDTO::default()
    };
    assert_eq!(
        printer.validate(None).unwrap_err(),
        "Choose a folder for DriveWire print jobs"
    );
}

#[test]
fn folder_output_with_a_folder_validates() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::Folder,
        folder: "prints".into(),
        ..DriveWirePrinterDTO::default()
    };
    assert!(printer.validate(None).is_ok());
}

#[test]
fn printer_port_without_a_serial_sink_fails_validation() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::PrinterPort,
        ..DriveWirePrinterDTO::default()
    };
    let error = printer.validate(None).unwrap_err();
    assert!(error.contains("Ports"), "{error}");
}

#[test]
fn printer_port_with_a_serial_sink_validates() {
    let printer = DriveWirePrinterDTO {
        output: PrintOutputDTO::PrinterPort,
        ..DriveWirePrinterDTO::default()
    };
    assert!(printer.validate(Some(SerialDTO::Printer)).is_ok());
}

#[test]
fn off_validates_regardless_of_folder_or_serial() {
    assert!(DriveWirePrinterDTO::default().validate(None).is_ok());
}

#[test]
fn printer_validation_is_skipped_while_drivewire_is_disabled() {
    let mut def = parsed();
    def.drivewire.enabled = false;
    def.drivewire.printer.folder.clear();
    assert!(def.validate_drivewire().is_ok());
}

#[test]
fn printer_validation_runs_while_drivewire_is_enabled() {
    let mut def = parsed();
    def.drivewire.printer.folder.clear();
    let error = def.validate_drivewire().unwrap_err();
    assert_eq!(error, "Choose a folder for DriveWire print jobs");
    assert!(def.to_machine_config().is_err());
}
