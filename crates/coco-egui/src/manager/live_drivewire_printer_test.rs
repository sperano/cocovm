//! Saved `[drivewire.printer]` edits reconcile into a Running VM, and a
//! printer-port output attaches the `[ports].serial` sink it needs.

use std::path::Path;

use coco_core::drivewire::{PrintFormat, PrintOutput};

use super::tests::{running_manager, vm};
use super::*;
use crate::machine_def::{self, tests::TempDir};

fn folder_printer(folder: &Path) -> machine_def::DriveWirePrinterDTO {
    machine_def::DriveWirePrinterDTO {
        output: machine_def::PrintOutputDTO::Folder,
        folder: folder.display().to_string(),
        format: machine_def::PrintFormatDTO::Text,
    }
}

#[test]
fn a_saved_printer_setting_switches_the_running_output() {
    let dir = TempDir::new("live-dw-printer-switch");
    let mut manager = running_manager(dir.path(), None);
    manager.entries[0].def.drivewire.enabled = true;
    manager.apply_live_drivewire(0);
    assert_eq!(
        vm(&manager)
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .print_output(),
        &PrintOutput::Off
    );

    let folder = dir.path().join("prints");
    manager.entries[0].def.drivewire.printer = folder_printer(&folder);
    manager.apply_live_drivewire(0);
    assert_eq!(
        vm(&manager)
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .print_output(),
        &PrintOutput::Folder {
            dir: folder,
            format: PrintFormat::Text,
        }
    );
}

#[test]
fn an_unrelated_drivewire_save_leaves_the_printer_output_as_configured() {
    let dir = TempDir::new("live-dw-printer-unrelated");
    let mut manager = running_manager(dir.path(), None);
    manager.entries[0].def.drivewire.enabled = true;
    let folder = dir.path().join("prints");
    manager.entries[0].def.drivewire.printer = folder_printer(&folder);
    manager.apply_live_drivewire(0);

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);
    assert_eq!(
        vm(&manager)
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .print_output(),
        &PrintOutput::Folder {
            dir: folder,
            format: PrintFormat::Text,
        }
    );
}

#[test]
fn a_printer_port_output_attaches_the_ports_sink_the_vm_launched_without() {
    let dir = TempDir::new("live-dw-printer-port");
    let mut manager = running_manager(dir.path(), None);
    assert!(!vm(&manager).has_printer_sink());
    let def = &mut manager.entries[0].def;
    def.drivewire.enabled = true;
    def.ports.serial = Some(machine_def::SerialDTO::Dmp130);
    def.drivewire.printer.output = machine_def::PrintOutputDTO::PrinterPort;

    manager.apply_live_drivewire(0);

    assert!(manager.entries[0].launch_error.is_none());
    let paper = vm(&manager).machine.bus.bitbanger.printer_handle();
    assert_eq!(
        paper.map(|paper| paper.model()),
        Some(coco_core::dmp::DmpModel::Dmp130)
    );
    assert_eq!(
        vm(&manager)
            .machine
            .bus
            .drivewire
            .as_ref()
            .unwrap()
            .print_output(),
        &PrintOutput::PrinterPort
    );
}

#[test]
fn a_printer_port_output_keeps_a_printer_attached_from_the_menu() {
    let dir = TempDir::new("live-dw-printer-keep");
    let mut manager = running_manager(dir.path(), None);
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .attach_printer(coco_core::dmp::DmpModel::Dmp105);
    let def = &mut manager.entries[0].def;
    def.drivewire.enabled = true;
    def.ports.serial = Some(machine_def::SerialDTO::Dmp130);
    def.drivewire.printer.output = machine_def::PrintOutputDTO::PrinterPort;

    manager.apply_live_drivewire(0);

    let paper = vm(&manager).machine.bus.bitbanger.printer_handle();
    assert_eq!(
        paper.map(|paper| paper.model()),
        Some(coco_core::dmp::DmpModel::Dmp105)
    );
}

#[test]
fn a_later_save_does_not_restart_a_capture_stopped_from_the_menu() {
    let dir = TempDir::new("live-dw-printer-stopped");
    let mut manager = running_manager(dir.path(), None);
    let def = &mut manager.entries[0].def;
    def.drivewire.enabled = true;
    def.ports.serial = Some(machine_def::SerialDTO::Dmp130);
    def.drivewire.printer.output = machine_def::PrintOutputDTO::PrinterPort;
    manager.apply_live_drivewire(0);
    let running = manager.entries[0].vm.as_mut().unwrap();
    running.stop_print_capture();
    running.cart_error = Some("earlier error".to_string());

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);

    assert!(!vm(&manager).has_printer_sink());
    assert_eq!(vm(&manager).cart_error.as_deref(), Some("earlier error"));
}

#[test]
fn enabling_drivewire_with_a_printer_port_output_attaches_the_ports_sink() {
    let dir = TempDir::new("live-dw-printer-enable");
    let mut manager = running_manager(dir.path(), None);
    manager.entries[0].def.drivewire.printer.output = machine_def::PrintOutputDTO::PrinterPort;
    manager.apply_live_drivewire(0);
    assert!(!vm(&manager).has_printer_sink());

    let def = &mut manager.entries[0].def;
    def.ports.serial = Some(machine_def::SerialDTO::Dmp130);
    def.drivewire.enabled = true;
    manager.apply_live_drivewire(0);

    assert!(vm(&manager).has_printer_sink());
}

/// A running VM whose print output is already the printer port, with no
/// `[ports].serial` sink and a retry due, as a failed live attach of
/// the file sink leaves it. The file sink itself can't be made to fail here:
/// it resolves under the real data directory.
fn manager_with_a_port_sink_to_retry(dir: &Path) -> ManagerApp {
    let mut manager = running_manager(dir, None);
    let def = &mut manager.entries[0].def;
    def.drivewire.enabled = true;
    def.drivewire.printer.output = machine_def::PrintOutputDTO::PrinterPort;
    manager.apply_live_drivewire(0);
    let running = manager.entries[0].vm.as_mut().unwrap();
    assert!(!running.has_printer_sink());
    running.dw_port_sink_retry = true;
    manager.entries[0].def.ports.serial = Some(machine_def::SerialDTO::Dmp130);
    manager
}

#[test]
fn a_failed_port_sink_attach_is_retried_on_the_next_save() {
    let dir = TempDir::new("live-dw-printer-retry");
    let mut manager = manager_with_a_port_sink_to_retry(dir.path());

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);

    assert!(vm(&manager).has_printer_sink());
    assert!(!vm(&manager).dw_port_sink_retry);
}

#[test]
fn a_capture_stopped_from_the_menu_ends_the_retry() {
    let dir = TempDir::new("live-dw-printer-retry-stopped");
    let mut manager = manager_with_a_port_sink_to_retry(dir.path());
    let running = manager.entries[0].vm.as_mut().unwrap();
    running.start_print_capture(dir.path().join("menu.txt"));
    running.stop_print_capture();

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);

    assert!(!vm(&manager).has_printer_sink());
    assert!(!vm(&manager).dw_port_sink_retry);
}

#[test]
fn a_printer_attached_from_the_menu_ends_the_retry() {
    let dir = TempDir::new("live-dw-printer-retry-menu");
    let mut manager = manager_with_a_port_sink_to_retry(dir.path());
    manager.entries[0]
        .vm
        .as_mut()
        .unwrap()
        .attach_printer(coco_core::dmp::DmpModel::Dmp105);
    assert!(!vm(&manager).dw_port_sink_retry);

    manager.entries[0].def.drivewire.hdbdos_mode = true;
    manager.apply_live_drivewire(0);

    let paper = vm(&manager).machine.bus.bitbanger.printer_handle();
    assert_eq!(
        paper.map(|paper| paper.model()),
        Some(coco_core::dmp::DmpModel::Dmp105)
    );
}
