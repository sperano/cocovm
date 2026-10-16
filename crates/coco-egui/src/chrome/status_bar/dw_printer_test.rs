use coco_core::drivewire::{DWServer, PrintOutput, opcode};

use super::*;

fn print_and_flush(dw: &mut DWServer, text: &[u8]) {
    for &byte in text {
        dw.data_write(opcode::PRINT, 0);
        dw.data_write(byte, 0);
    }
    dw.data_write(opcode::PRINTFLUSH, 0);
}

#[test]
fn an_unused_off_printer_has_no_hover_lines() {
    let dw = DWServer::new();
    assert_eq!(hover_lines(&dw), "");
}

#[test]
fn a_non_off_output_shows_even_with_nothing_printed_yet() {
    let mut dw = DWServer::new();
    dw.set_print_output(PrintOutput::PrinterPort);
    assert_eq!(hover_lines(&dw), "\nPrinter: 0 printed, 0 waiting");
}

#[test]
fn a_printed_job_stays_reported_after_switching_back_to_off() {
    let mut dw = DWServer::new();
    dw.set_print_output(PrintOutput::PrinterPort);
    print_and_flush(&mut dw, b"hi");
    // Like `SystemBus::deliver_drivewire_print`, draining the port job is what counts it.
    while dw.take_port_print_job().is_some() {}
    dw.set_print_output(PrintOutput::Off);
    assert_eq!(hover_lines(&dw), "\nPrinter: 1 printed, 0 waiting");
}

#[test]
fn an_error_adds_a_second_line() {
    let mut dw = DWServer::new();
    dw.set_print_output(PrintOutput::Folder {
        dir: std::path::PathBuf::from("/does/not/exist/blocked-by-a-missing-parent"),
        format: coco_core::drivewire::PrintFormat::Text,
    });
    print_and_flush(&mut dw, b"hi");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        dw.poll_host();
        if dw.print_diagnostics().last_error.is_some() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "write never failed");
        std::thread::yield_now();
    }
    let hover = hover_lines(&dw);
    assert!(hover.contains("Printer: 0 printed, 1 waiting"), "{hover}");
    assert!(hover.contains("Printer error: "), "{hover}");
}
