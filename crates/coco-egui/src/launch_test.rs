//! `launch_machine` tests for the peripherals/ports this module mounts
//! beyond the base `CocoApp::new` construction — the RS-232 Pak, the
//! printer serial sink, and the MPI/RS-232 conflict `check_cartridge_port`
//! rejects. Reads the real `roms/coco3.rom` (git-ignored, local-only), like
//! every other test in this crate that boots a real machine
//! (`save_state_test.rs`'s doc comment).

use coco_core::MachineConfig;

use crate::machine_def::MachineDef;

/// A minimal CoCo 3 definition ([`MachineConfig::default`]) with no media
/// and no peripherals — callers flip on just the `[peripherals]`/`[ports]`
/// fields the test cares about.
fn base_def() -> MachineDef {
    MachineDef::from_config("Launch Test".to_string(), None, &MachineConfig::default())
}

/// `[peripherals].rs232 = true` mounts the Deluxe RS-232 Pak straight into
/// the cartridge port (`mount_peripherals`'s non-MPI `rs232` arm →
/// `CocoApp::insert_rs232`).
#[test]
fn rs232_def_mounts_the_pak() {
    let mut def = base_def();
    def.peripherals.rs232 = true;

    let mut app = super::launch_machine(&def, "launch-test-rs232")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        app.machine.bus.cart.as_deluxe_rs232().is_some(),
        "the cartridge port should hold the Deluxe RS-232 Pak"
    );
}

/// `[ports].serial = "printer"` attaches a DMP-105 to the bit-banger with
/// the paper window closed (`mount_serial`'s `Printer` arm) — the window
/// shows the accumulating output once opened, per `paper_view`'s
/// sink-ownership doc.
#[test]
fn printer_def_attaches_the_paper_window_handle() {
    let mut def = base_def();
    def.ports.serial = Some(crate::machine_def::SerialDTO::Printer);

    let app = super::launch_machine(&def, "launch-test-printer")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        app.paper_window.handle.is_some(),
        "the paper window should hold a DMP-105 handle"
    );
    assert!(!app.paper_window.open, "attached with the window closed");
}

/// `[ui].joy_left`/`joy_right` reach the launched `CocoApp`'s
/// `joysticks.sources`, indexed by `coco_core::joystick::{RIGHT, LEFT}` —
/// the definition's *starting* state, same as `aspect_correct`/`kb_mode`
/// (`launch_machine`'s doc comment).
#[test]
fn joy_sources_def_reaches_the_app() {
    let mut def = base_def();
    def.ui.joy_left = crate::machine_def::JoySourceDTO::Keys;
    def.ui.joy_right = crate::machine_def::JoySourceDTO::Gamepad;

    let app = super::launch_machine(&def, "launch-test-joy")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::LEFT],
        crate::joy::JoySource::Keys
    );
    assert_eq!(
        app.joysticks.sources[coco_core::joystick::RIGHT],
        crate::joy::JoySource::Gamepad
    );
}

/// `mpi = true` + `rs232 = true` is rejected outright — the pak has no
/// MultiPak-slot support yet, so an MPI can't lift the one-peripheral limit
/// for it the way it does for the FD-502/RTC (`check_cartridge_port`'s doc).
#[test]
fn mpi_and_rs232_together_errors() {
    let mut def = base_def();
    def.peripherals.mpi = true;
    def.peripherals.rs232 = true;

    let err = super::launch_machine(&def, "launch-test-mpi-rs232")
        .err()
        .expect("mpi + rs232 must be rejected");
    assert!(
        err.contains("MultiPak"),
        "error should explain the MPI-slot gap: {err}"
    );
}
