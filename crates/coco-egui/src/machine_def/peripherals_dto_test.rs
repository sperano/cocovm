//! `[peripherals]` schema tests: the pre-enum legacy format's fatal rejection,
//! and the MultiPak's `slots` requirement — split out of `machine_def_test.rs`
//! once that file grew past the project's ~500-line ceiling.

use std::fs;

use super::{CartridgeDTO, SlotDTO};
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
            slots: std::array::from_fn(|_| SlotDTO::Empty)
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
    };
    save(dir.path(), "mpi-empty", &def).expect("save should succeed");

    let loaded = load_all(dir.path()).expect("an all-empty MPI must load back");
    assert_eq!(loaded.len(), 1);
    assert_eq!(&loaded[0].1, &def);
}
