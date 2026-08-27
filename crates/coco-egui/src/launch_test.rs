//! `launch_machine` tests for the peripherals/ports this module mounts
//! beyond the base `CocoApp::new` construction — the RS-232 Pak, the
//! printer serial sink, and disk media with no reachable controller. Boots
//! through `launch_machine`'s production ROM resolution, so it reads the
//! installed `coco3.rom` (`installed_roms_dir`, populated by
//! `ensure_assets` — `save_state_test.rs`'s doc comment).

use coco_core::MachineConfig;

use crate::RS232Endpoint;
use crate::machine_def::{CartridgeDTO, MachineDef, RS232EndpointDTO, SlotDTO};

/// A minimal CoCo 3 definition ([`MachineConfig::default`]) with no media
/// and no peripherals — callers flip on just the `[peripherals]`/`[ports]`
/// fields the test cares about.
fn base_def() -> MachineDef {
    MachineDef::from_config("Launch Test".to_string(), None, &MachineConfig::default())
}

/// `[peripherals].cartridge = { kind = "rs232" }` mounts the Deluxe RS-232
/// Pak straight into the cartridge port, on the loopback endpoint — both the
/// explicit choice here and the DTO's own default with no `endpoint` key
/// (`peripherals_dto_test.rs` covers the bare-key parse itself).
#[test]
fn rs232_def_mounts_the_pak_on_loopback() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::RS232 {
        endpoint: RS232EndpointDTO::Loopback,
    };

    let mut app = super::launch_machine(&def, "launch-test-rs232")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        app.machine.bus.cart.as_deluxe_rs232().is_some(),
        "the cartridge port should hold the Deluxe RS-232 Pak"
    );
    assert!(
        matches!(app.rs232, Some(RS232Endpoint::Loopback)),
        "loopback is the pak's default endpoint"
    );
}

/// `[peripherals].cartridge = { kind = "rs232", endpoint = { kind = "tcp", listen = ... } }`
/// binds the pak's serial line to a TCP listener at the configured address.
/// Binds to port 0 (OS-assigned) so the test can't collide with another
/// listener, and drops `app` (closing the socket) before returning.
#[test]
fn rs232_tcp_endpoint_binds_the_configured_address() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::RS232 {
        endpoint: RS232EndpointDTO::TCP {
            listen: "127.0.0.1:0".to_string(),
        },
    };

    let app = super::launch_machine(&def, "launch-test-rs232-tcp")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    match &app.rs232 {
        Some(RS232Endpoint::TCP(addr)) => {
            assert!(
                addr.starts_with("127.0.0.1:"),
                "the bound address should be on 127.0.0.1: {addr}"
            );
        }
        other => panic!(
            "expected a bound TCP endpoint, got {}",
            other.as_ref().map_or("none".to_string(), |e| e.label())
        ),
    }
}

/// `[ports].serial = "printer"` attaches a DMP-105 to the bit-banger with
/// the paper window closed; it shows accumulating output once opened.
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
/// `joysticks.sources` as the definition's starting state.
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

/// `disk0` with no reachable disk controller (no bare FD-502, no MPI slot
/// holding one) is a fatal error naming the fix — nothing implies a
/// controller anymore.
#[test]
fn disk_media_with_no_controller_errors() {
    let mut def = base_def();
    def.media.disk0 = Some("dev.dsk".to_string());

    let err = super::launch_machine(&def, "launch-test-disk-no-controller")
        .err()
        .expect("disk media with no controller must be rejected");
    assert!(
        err.contains("disk controller"),
        "error should name the fix: {err}"
    );
}

/// Writes a one-track blank floppy image and returns a definition mounting it as
/// `disk0` with `cartridge` as the port occupant.
fn disk0_def(dir: &crate::machine_def::tests::TempDir, cartridge: CartridgeDTO) -> MachineDef {
    use coco_core::fdc;

    let disk_path = dir.path().join("dev.dsk");
    let sector_size = 128usize << fdc::DEFAULT_SECTOR_SIZE_CODE;
    let one_track = fdc::DEFAULT_SECTORS_PER_TRACK * sector_size * fdc::DEFAULT_SIDES;
    std::fs::write(&disk_path, vec![0u8; one_track]).expect("write disk fixture");

    let mut def = base_def();
    def.media.disk0 = Some(disk_path.display().to_string());
    def.peripherals.cartridge = cartridge;
    def
}

/// Launches `def` and asserts the floppy actually landed in drive 0 — the mount
/// happens after the controller is installed, so a silently skipped mount would
/// otherwise still return `Ok` with empty drives.
fn assert_disk0_mounted(def: &MachineDef, slug: &str) {
    let mut app =
        super::launch_machine(def, slug).unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert_eq!(app.cart_error, None, "no mount error expected");
    assert_eq!(
        app.disk_paths[0].as_deref().and_then(|p| p.file_name()),
        Some(std::ffi::OsStr::new("dev.dsk")),
        "drive 0 should hold the definition's disk0"
    );
    let cart = app
        .machine
        .bus
        .cart
        .as_disk_cart()
        .expect("an FD-502 should be reachable after launch");
    assert!(
        cart.is_mounted(0),
        "the floppy should be in the controller's drive 0"
    );
}

/// The same disk media launches fine once an MPI slot holds the FD-502.
#[test]
fn disk_media_with_mpi_fd502_launches() {
    let dir = crate::machine_def::tests::TempDir::new("launch-disk-mpi-fd502");
    let def = disk0_def(
        &dir,
        CartridgeDTO::MPI {
            slots: [
                SlotDTO::FD502,
                SlotDTO::Empty,
                SlotDTO::Empty,
                SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        },
    );
    assert_disk0_mounted(&def, "launch-test-disk-mpi-fd502");
}

/// A bare-port FD-502 mounts disk media too — the controller is inserted by
/// `mount_peripherals` first, then the floppy.
#[test]
fn disk_media_with_bare_fd502_launches() {
    let dir = crate::machine_def::tests::TempDir::new("launch-disk-bare-fd502");
    let def = disk0_def(&dir, CartridgeDTO::FD502);
    assert_disk0_mounted(&def, "launch-test-disk-bare-fd502");
}

/// `[peripherals].cartridge.switch = 2` (1-based, the UI's "Slot 2") moves the
/// MPI's front-panel switch to 0-based slot 1 — both the frontend's mirror
/// ([`crate::MPIState::switch`]) and the core `MultiPak` itself.
#[test]
fn mpi_switch_config_sets_the_front_panel_switch() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: std::array::from_fn(|_| SlotDTO::Empty),
        switch: 2,
    };

    let mut app = super::launch_machine(&def, "launch-test-mpi-switch")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert_eq!(
        app.mpi.as_ref().map(|m| m.switch),
        Some(1),
        "the frontend's switch mirror is 0-based"
    );
    let mp = app
        .machine
        .bus
        .cart
        .as_multipak()
        .expect("the MPI should be installed");
    assert_eq!(
        mp.switch_slot(),
        1,
        "the core MultiPak must read back the same slot"
    );
}

/// `[peripherals].cartridge = { kind = "rompak", autostart = false }` leaves the mounted
/// ROM Pak's CART* line untied from Q — it must not autostart.
#[test]
fn rompak_autostart_false_does_not_tie_cart_line_to_q() {
    let dir = crate::machine_def::tests::TempDir::new("launch-rompak-no-autostart");
    let rom_path = dir.path().join("game.rom");
    std::fs::write(&rom_path, vec![0x11u8; 0x4000]).expect("write ROM pak fixture");

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::ROMPak {
        path: rom_path.display().to_string(),
        autostart: false,
    };

    let app = super::launch_machine(&def, "launch-test-rompak-no-autostart")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        !app.machine.bus.cart.cart_line_ties_q(),
        "autostart = false must not tie CART* to Q"
    );
}

/// `[peripherals].cartridge` naming an MPI slot's ROM Pak with `autostart = false` carries the
/// same flag through `mpi_insert_rompak`.
#[test]
fn mpi_slot_rompak_autostart_false_does_not_tie_cart_line_to_q() {
    let dir = crate::machine_def::tests::TempDir::new("launch-mpi-rompak-no-autostart");
    let rom_path = dir.path().join("game.rom");
    std::fs::write(&rom_path, vec![0x11u8; 0x4000]).expect("write ROM pak fixture");

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::ROMPak {
                path: rom_path.display().to_string(),
                autostart: false,
            },
            SlotDTO::Empty,
            SlotDTO::Empty,
            SlotDTO::Empty,
        ],
        switch: 1,
    };

    let app = super::launch_machine(&def, "launch-test-mpi-rompak-no-autostart")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        !app.machine.bus.cart.cart_line_ties_q(),
        "autostart = false must not tie CART* to Q"
    );
}
