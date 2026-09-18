//! `launch_machine` tests for the peripherals/ports this module mounts
//! beyond the base `CocoApp::new` construction — the RS-232 Pak, the
//! printer serial sink, and disk media with no reachable controller. Boots
//! through `launch_machine`'s production ROM resolution, so it reads the
//! installed `coco3.rom` (`installed_roms_dir`, populated by the first-run
//! asset download — `save_state_test.rs`'s doc comment).

use coco_core::MachineConfig;
use mc6809::Bus;

use crate::RS232Endpoint;
use crate::machine_def::{CartridgeDTO, MachineDef, RS232EndpointDTO, SlotDTO};

/// A minimal CoCo 3 definition ([`MachineConfig::default`]) with no media
/// and no peripherals — callers flip on only the `[peripherals]`/`[ports]`
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

/// A listen address already bound by something else must not fail the whole launch —
/// every definition without an explicit `listen` shares the same default
/// ([`crate::RS232_TCP_DEFAULT_ADDR`]), so a second RS-232 machine would otherwise never
/// launch. The pak falls back to loopback (still usable) and the collision is reported
/// through the non-fatal status-bar toast instead.
#[test]
fn rs232_tcp_bind_failure_falls_back_to_loopback_with_a_toast() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a blocking listener");
    let addr = blocker.local_addr().expect("local_addr").to_string();

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::RS232 {
        endpoint: RS232EndpointDTO::TCP {
            listen: addr.clone(),
        },
    };

    let mut app = super::launch_machine(&def, "launch-test-rs232-tcp-collision")
        .unwrap_or_else(|e| panic!("a bind failure must not fail the whole launch: {e}"));
    assert!(
        matches!(app.rs232, Some(RS232Endpoint::Loopback)),
        "a failed bind must fall back to loopback"
    );
    assert_eq!(
        app.cart_error, None,
        "a bind failure must not be promoted to a fatal launch error"
    );
    let toast = app
        .toast_message()
        .expect("the bind failure should be reported through the toast");
    assert!(
        toast.contains(&addr),
        "the toast should name the address that failed to bind: {toast}"
    );

    drop(blocker);
}

/// A bind failure is demoted to a toast, but an earlier slot's fatal
/// `cart_error` (a missing Disk BASIC ROM, say) must survive the demotion.
#[test]
fn rs232_bind_failure_toast_does_not_swallow_a_prior_cart_error() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a blocking listener");
    let addr = blocker.local_addr().expect("local_addr").to_string();

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::RS232 {
        endpoint: RS232EndpointDTO::Loopback,
    };
    let mut app = super::launch_machine(&def, "launch-test-rs232-prior-error")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));

    const PRIOR: &str = "could not read Disk BASIC ROM";
    app.cart_error = Some(PRIOR.to_string());
    super::apply_rs232_endpoint(
        &mut app,
        RS232EndpointDTO::TCP {
            listen: addr.clone(),
        },
    );

    assert_eq!(
        app.cart_error.as_deref(),
        Some(PRIOR),
        "the earlier fatal error must not be consumed by the endpoint toast"
    );
    let toast = app
        .toast_message()
        .expect("the bind failure still reports a toast");
    assert!(
        toast.contains(&addr),
        "toast should name the address: {toast}"
    );

    drop(blocker);
}

/// Each printer definition attaches the chosen model with its paper window closed.
#[test]
fn printer_def_attaches_the_selected_paper_window_handle() {
    use crate::machine_def::SerialDTO;
    use coco_core::dmp::DmpModel;

    for (serial, model) in [
        (SerialDTO::Printer, DmpModel::Dmp105),
        (SerialDTO::Dmp130, DmpModel::Dmp130),
    ] {
        let mut def = base_def();
        def.ports.serial = Some(serial);
        let app = super::launch_machine(&def, "launch-test-printer")
            .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
        assert_eq!(app.paper_window.handle.as_ref().unwrap().model(), model);
        assert_eq!(
            app.machine.bus.bitbanger.printer_handle().unwrap().model(),
            model
        );
        assert!(!app.paper_window.open, "attached with the window closed");
    }
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

/// The CoCo Max Hi-Res Input Module requires CoCo 1/2 hardware: the CoCo 3's
/// GIME owns its `$FF90-$FF97` ADC window, so `launch_machine` must refuse
/// it on `base_def`'s CoCo 3, the same way `insert_cocomax` refuses it at
/// runtime.
#[test]
fn cocomax_refuses_a_coco3_machine() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::CoCoMax;

    let err = super::launch_machine(&def, "launch-test-cocomax-coco3")
        .err()
        .expect("the CoCo Max module must be refused on a CoCo 3");
    assert!(
        err.contains("CoCo 1") && err.contains("CoCo 2"),
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
                SlotDTO::FD502 {
                    dos_rom: Default::default(),
                },
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
    let def = disk0_def(
        &dir,
        CartridgeDTO::FD502 {
            dos_rom: Default::default(),
        },
    );
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

#[test]
fn banked_rompak_mounts_without_games_master_hardware() {
    const BANKED_ROM_SIZE: usize = 0x8000;
    let dir = crate::machine_def::tests::TempDir::new("launch-banked-rompak");
    let rom_path = dir.path().join("mind-roll.rom");
    std::fs::write(&rom_path, vec![0x11u8; BANKED_ROM_SIZE]).expect("write banked ROM Pak fixture");

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::BankedROMPak {
        path: rom_path.display().to_string(),
        autostart: true,
    };

    let app = super::launch_machine(&def, "launch-test-banked-rompak")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(matches!(
        &app.machine.bus.cart,
        coco_core::cart::Cart::BankedROMPak(_)
    ));
    assert!(!app.machine.bus.cart.contains_games_master());
}

#[test]
fn mpi_slot_mounts_banked_rompak_without_games_master_hardware() {
    const BANKED_ROM_SIZE: usize = 0x10000;
    const BANKED_SLOT: u8 = 0;
    let dir = crate::machine_def::tests::TempDir::new("launch-mpi-banked-rompak");
    let rom_path = dir.path().join("predator.rom");
    std::fs::write(&rom_path, vec![0x22u8; BANKED_ROM_SIZE]).expect("write banked ROM Pak fixture");

    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::BankedROMPak {
                path: rom_path.display().to_string(),
                autostart: true,
            },
            SlotDTO::Empty,
            SlotDTO::Empty,
            SlotDTO::Empty,
        ],
        switch: usize::from(BANKED_SLOT) + 1,
    };

    let mut app = super::launch_machine(&def, "launch-test-mpi-banked-rompak")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    let (_, slot_cart) = app
        .machine
        .bus
        .cart
        .slots_mut()
        .into_iter()
        .find(|(slot, _)| *slot == Some(BANKED_SLOT))
        .expect("banked slot must exist");
    assert!(matches!(slot_cart, coco_core::cart::Cart::BankedROMPak(_)));
    assert!(!app.machine.bus.cart.contains_games_master());
}

/// An MPI slot may hold the Deluxe RS-232 Pak: it decodes its ACIA at
/// `$FF68-$FF6B` off the full address bus itself, so it's reachable
/// regardless of the MPI's switch/`$FF7F` selection — here the pak is in
/// slot 1 (index 0) while the switch (and an FD-502) sit on slot 4.
#[test]
fn mpi_slot_rs232_reaches_the_acia_regardless_of_switch() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::RS232 {
                endpoint: RS232EndpointDTO::Loopback,
            },
            SlotDTO::Empty,
            SlotDTO::Empty,
            SlotDTO::FD502 {
                dos_rom: Default::default(),
            },
        ],
        switch: 4,
    };

    let mut app = super::launch_machine(&def, "launch-test-mpi-rs232-slot")
        .unwrap_or_else(|e| panic!("launch should succeed: {e}"));
    assert!(
        matches!(app.rs232, Some(RS232Endpoint::Loopback)),
        "loopback is the pak's default endpoint"
    );
    assert!(
        app.machine.bus.cart.as_deluxe_rs232().is_some(),
        "the RS-232 pak nested in slot 1 must be reachable through the MultiPak"
    );

    // $FF68-$FF6B is outside the SCS* window and ungated (`bus/io.rs`'s `io_read`/`io_write`
    // route $FF60-$FF7E straight to the cart, no INIT0 MC2 gate needed) -- the control
    // register (offset 3) is a plain get/set with no read side effect.
    const CONTROL_REG: u16 = coco_core::rs232::ACIA_BASE + 3;
    app.machine.bus.write(CONTROL_REG, 0x1F);
    let via_bus = app.machine.bus.read(CONTROL_REG);
    let via_pak = app
        .machine
        .bus
        .cart
        .as_deluxe_rs232()
        .expect("the pak is still reachable")
        .acia()
        .read(3);
    assert_eq!(
        via_bus, 0x1F,
        "the bus read must reach the ACIA's control register"
    );
    assert_eq!(
        via_bus, via_pak,
        "the bus read must agree with the pak's own status-register read"
    );
}

/// A TCP endpoint on an MPI-slotted RS-232 Pak binds the same way as a
/// bare-port one (`rs232_tcp_endpoint_binds_the_configured_address` earlier).
#[test]
fn mpi_slot_rs232_tcp_endpoint_binds_the_configured_address() {
    let mut def = base_def();
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: [
            SlotDTO::RS232 {
                endpoint: RS232EndpointDTO::TCP {
                    listen: "127.0.0.1:0".to_string(),
                },
            },
            SlotDTO::Empty,
            SlotDTO::Empty,
            SlotDTO::Empty,
        ],
        switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
    };

    let app = super::launch_machine(&def, "launch-test-mpi-rs232-slot-tcp")
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
