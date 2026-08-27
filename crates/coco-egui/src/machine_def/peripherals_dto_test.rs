//! `[peripherals]` schema tests: the pre-enum legacy format's fatal rejection,
//! and the MultiPak's `slots` requirement — split out of `machine_def_test.rs`
//! once that file grew past the project's ~500-line ceiling.

use std::fs;

use super::{CartridgeDTO, RS232EndpointDTO, SlotDTO};
use crate::MPI_SLOT_COUNT;
use crate::machine_def::tests::TempDir;
use crate::machine_def::{MachineDef, load_all, save};

/// A pre-enum `[peripherals]` (the four independent booleans) must fail to
/// load: `cartridge` has no `#[serde(default)]`, so the missing key is a
/// parse error rather than a silently-empty port that drops the file's
/// `mpi = true`. There's no migration by design.
#[test]
fn legacy_boolean_peripherals_fails_to_load() {
    let dir = TempDir::new("legacy-peripherals");
    fs::write(
        dir.path().join("legacy.toml"),
        r#"
schema = 1
name = "Legacy Peripherals"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
mpi = true
rtc = true
fd502 = false
rs232 = false
"#,
    )
    .unwrap();
    let err = load_all(dir.path()).expect_err("a pre-enum [peripherals] must fail the load");
    assert!(
        err.contains("cartridge"),
        "error should name the missing field: {err}"
    );
}

/// `[peripherals].cartridge = { kind = "mpi" }` with no `slots` key must
/// fail to load — `slots` is a required field of `CartridgeDTO::MPI`, so
/// serde's own missing-field error catches it directly (no raw-table check
/// needed for this case, unlike the wrong-length one below).
#[test]
fn mpi_cartridge_with_no_slots_fails_to_load() {
    let dir = TempDir::new("mpi-no-slots");
    fs::write(
        dir.path().join("mpi-bare.toml"),
        r#"
schema = 1
name = "MPI, No Slots"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
cartridge = { kind = "mpi" }
"#,
    )
    .unwrap();
    let err = load_all(dir.path()).expect_err("an MPI with no slots key must fail the load");
    assert!(
        err.contains("slots"),
        "error should name the missing key: {err}"
    );
}

/// A `slots` array of the wrong length is rejected with a message naming
/// the expected count, not serde's generic length error.
#[test]
fn mpi_cartridge_with_wrong_slot_count_fails_to_load() {
    for count in [MPI_SLOT_COUNT - 1, MPI_SLOT_COUNT + 1] {
        let dir = TempDir::new(&format!("mpi-{count}-slots"));
        let slots = vec!["{ kind = \"empty\" }"; count].join(", ");
        fs::write(
            dir.path().join("mpi.toml"),
            format!(
                r#"
schema = 1
name = "MPI, Wrong Slot Count"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
cartridge = {{ kind = "mpi", slots = [{slots}] }}
"#
            ),
        )
        .unwrap();
        let err = load_all(dir.path()).expect_err("a wrong-length slots array must fail the load");
        assert!(
            err.contains(&format!("lists {count} slots")) && err.contains("exactly 4"),
            "error should name the actual and expected counts: {err}"
        );
    }
}

/// An MPI with `slots` explicitly all `empty` loads fine — the fatal case is
/// a missing `slots` key, not an intentionally empty MultiPak.
#[test]
fn mpi_cartridge_with_explicit_empty_slots_loads() {
    let dir = TempDir::new("mpi-empty-slots");
    fs::write(
        dir.path().join("mpi-empty.toml"),
        r#"
schema = 1
name = "MPI, Empty Slots"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
cartridge = { kind = "mpi", slots = [{ kind = "empty" }, { kind = "empty" }, { kind = "empty" }, { kind = "empty" }] }
"#,
    )
    .unwrap();
    let loaded = load_all(dir.path()).expect("an explicitly-empty MPI loadout must load");
    assert_eq!(
        loaded[0].1.peripherals.cartridge,
        CartridgeDTO::MPI {
            slots: std::array::from_fn(|_| SlotDTO::Empty),
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// The blocking case the missing-`slots` check exists for: a two-click MPI
/// with nothing in any slot must save with all four slots explicit and
/// still load back, rather than round-tripping through a state `load_all`
/// then rejects as fatal.
#[test]
fn all_empty_mpi_round_trips() {
    let dir = TempDir::new("mpi-all-empty-roundtrip");
    let mut def = MachineDef::from_config(
        "All-Empty MPI".to_string(),
        None,
        &coco_core::MachineConfig::default(),
    );
    def.peripherals.cartridge = CartridgeDTO::MPI {
        slots: std::array::from_fn(|_| SlotDTO::Empty),
        switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
    };
    save(dir.path(), "mpi-empty", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("an all-empty MPI must load back");
    assert_eq!(loaded.len(), 1);
    assert_eq!(&loaded[0].1, &def);
}

/// `[peripherals].cartridge.switch` round-trips an explicit 1-based front-panel slot.
#[test]
fn mpi_explicit_switch_round_trips() {
    let dto: CartridgeDTO = toml::from_str(
        r#"
kind = "mpi"
switch = 2
slots = [{ kind = "empty" }, { kind = "empty" }, { kind = "empty" }, { kind = "empty" }]
"#,
    )
    .expect("a valid switch must parse");
    assert_eq!(
        dto,
        CartridgeDTO::MPI {
            slots: std::array::from_fn(|_| SlotDTO::Empty),
            switch: 2,
        }
    );
}

/// Each of `RS232EndpointDTO`'s three `endpoint` shapes round-trips.
#[test]
fn rs232_endpoint_kinds_round_trip() {
    let loopback: CartridgeDTO = toml::from_str(
        r#"
kind = "rs232"
endpoint = { kind = "loopback" }
"#,
    )
    .expect("loopback endpoint must parse");
    assert_eq!(
        loopback,
        CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::Loopback
        }
    );

    let tcp: CartridgeDTO = toml::from_str(
        r#"
kind = "rs232"
endpoint = { kind = "tcp", listen = "127.0.0.1:6551" }
"#,
    )
    .expect("tcp endpoint must parse");
    assert_eq!(
        tcp,
        CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::TCP {
                listen: "127.0.0.1:6551".to_string()
            }
        }
    );

    let pty: CartridgeDTO = toml::from_str(
        r#"
kind = "rs232"
endpoint = { kind = "pty" }
"#,
    )
    .expect("pty endpoint must parse");
    assert_eq!(
        pty,
        CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::PTY
        }
    );
}

/// `[peripherals].cartridge.autostart = false` round-trips for a ROM Pak.
#[test]
fn rompak_autostart_false_round_trips() {
    let dto: CartridgeDTO = toml::from_str(
        r#"
kind = "rompak"
path = "/paks/game.ccc"
autostart = false
"#,
    )
    .expect("autostart = false must parse");
    assert_eq!(
        dto,
        CartridgeDTO::ROMPak {
            path: "/paks/game.ccc".to_string(),
            autostart: false,
        }
    );
}

/// The legacy shapes from before `switch`/`endpoint` existed — `{ kind = "mpi", slots = [...] }`
/// with no `switch` key, and `{ kind = "rs232" }` with no `endpoint` key — still deserialize,
/// using the documented defaults (`peripherals_dto.rs`'s `default_mpi_switch`/`RS232EndpointDTO`'s
/// `Default`).
#[test]
fn legacy_mpi_and_rs232_shapes_use_the_documented_defaults() {
    let mpi: CartridgeDTO = toml::from_str(
        r#"
kind = "mpi"
slots = [{ kind = "empty" }, { kind = "empty" }, { kind = "empty" }, { kind = "empty" }]
"#,
    )
    .expect("a switch-less MPI must still parse");
    assert_eq!(
        mpi,
        CartridgeDTO::MPI {
            slots: std::array::from_fn(|_| SlotDTO::Empty),
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );

    let rs232: CartridgeDTO =
        toml::from_str(r#"kind = "rs232""#).expect("an endpoint-less rs232 must still parse");
    assert_eq!(
        rs232,
        CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::default()
        }
    );
}

/// An MPI slot's RS-232 Pak round-trips each endpoint kind, same as the bare-port
/// `CartridgeDTO::RS232` (`rs232_endpoint_kinds_round_trip` above).
#[test]
fn slot_rs232_endpoint_kinds_round_trip() {
    let dto: CartridgeDTO = toml::from_str(
        r#"
kind = "mpi"
slots = [
    { kind = "rs232", endpoint = { kind = "tcp", listen = "127.0.0.1:6551" } },
    { kind = "empty" },
    { kind = "empty" },
    { kind = "empty" },
]
"#,
    )
    .expect("a slotted rs232 with a tcp endpoint must parse");
    assert_eq!(
        dto,
        CartridgeDTO::MPI {
            slots: [
                SlotDTO::RS232 {
                    endpoint: RS232EndpointDTO::TCP {
                        listen: "127.0.0.1:6551".to_string()
                    }
                },
                SlotDTO::Empty,
                SlotDTO::Empty,
                SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// A legacy `{ kind = "rs232" }` slot with no `endpoint` key loads as loopback,
/// same as the bare-port shape (`legacy_mpi_and_rs232_shapes_use_the_documented_defaults`).
#[test]
fn slot_rs232_with_no_endpoint_loads_as_loopback() {
    let dto: CartridgeDTO = toml::from_str(
        r#"
kind = "mpi"
slots = [{ kind = "rs232" }, { kind = "empty" }, { kind = "empty" }, { kind = "empty" }]
"#,
    )
    .expect("an endpoint-less slotted rs232 must still parse");
    assert_eq!(
        dto,
        CartridgeDTO::MPI {
            slots: [
                SlotDTO::RS232 {
                    endpoint: RS232EndpointDTO::default()
                },
                SlotDTO::Empty,
                SlotDTO::Empty,
                SlotDTO::Empty,
            ],
            switch: crate::DEFAULT_MPI_SWITCH_SLOT + 1,
        }
    );
}

/// Two RS-232 Paks in the same MPI's `slots` are rejected at load time — they'd fight
/// over the shared ACIA at `$FF68`.
#[test]
fn mpi_cartridge_with_two_rs232_slots_fails_to_load() {
    let dir = TempDir::new("mpi-two-rs232");
    fs::write(
        dir.path().join("mpi.toml"),
        r#"
schema = 1
name = "MPI, Two RS-232"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
cartridge = { kind = "mpi", slots = [{ kind = "rs232" }, { kind = "rs232" }, { kind = "empty" }, { kind = "empty" }] }
"#,
    )
    .unwrap();
    let err = load_all(dir.path()).expect_err("two RS-232 slots must fail the load");
    assert!(
        err.contains("RS-232") && err.contains("$FF68"),
        "error should name the conflict: {err}"
    );
}

/// A `switch` outside 1..=4 is rejected with a message naming the valid range, not serde's
/// generic error.
#[test]
fn mpi_switch_out_of_range_fails_to_load() {
    for switch in [0, 5] {
        let dir = TempDir::new(&format!("mpi-switch-{switch}"));
        fs::write(
            dir.path().join("mpi.toml"),
            format!(
                r#"
schema = 1
name = "MPI, Bad Switch"

[hardware]
variant = "coco3"
ram = "512k"
video = "ntsc"
monitor = "rgb"

[peripherals]
cartridge = {{ kind = "mpi", switch = {switch}, slots = [{{ kind = "empty" }}, {{ kind = "empty" }}, {{ kind = "empty" }}, {{ kind = "empty" }}] }}
"#
            ),
        )
        .unwrap();
        let err = load_all(dir.path()).expect_err("an out-of-range switch must fail the load");
        assert!(
            err.contains("switch") && err.contains("1") && err.contains("4"),
            "error should name the actual and valid range: {err}"
        );
    }
}
