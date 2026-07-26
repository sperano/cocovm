//! Phase-1 smoke tests for the serde derive pass over `Machine` and its
//! whole device tree (`docs/plan-save-states.md`, `docs/plan-machine-persistence.md`).
//! Not the real save-state format yet — that's a later phase, once the CBOR
//! snapshot is wrapped in the container format the persistence plan
//! describes. This just proves the derive pass itself is trace-faithful: a
//! machine mid-BASIC-idle-loop round-trips through CBOR and keeps executing
//! identically to an un-serialized twin.

use std::path::PathBuf;

use coco_core::cart::{Cart, Cartridge};
use coco_core::{Machine, MachineConfig};
use mc6809::{MC6809, State};

fn load_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

fn boot_machine() -> Machine {
    Machine::new(MachineConfig::default(), load_rom())
}

/// CPU-visible state a lockstep comparison after each step must agree on:
/// every register, the total cycle count, and the run/halt state
/// ([`mc6809::State`]). Deliberately excludes anything that isn't part of
/// the architectural CPU state (e.g. the NMI-armed flag) — this is a
/// trace-identity check on execution, not a full-struct `PartialEq`.
#[derive(Debug, PartialEq)]
struct CpuSnapshot {
    a: u8,
    b: u8,
    x: u16,
    y: u16,
    u: u16,
    s: u16,
    pc: u16,
    dp: u8,
    cc: u8,
    cycles: u64,
    state: State,
}

impl CpuSnapshot {
    fn of(cpu: &MC6809) -> Self {
        Self {
            a: cpu.a,
            b: cpu.b,
            x: cpu.x,
            y: cpu.y,
            u: cpu.u,
            s: cpu.s,
            pc: cpu.pc,
            dp: cpu.dp,
            cc: cpu.cc,
            cycles: cpu.cycles,
            state: cpu.state,
        }
    }
}

/// Number of instructions to run before taking the snapshot — well into
/// BASIC's idle loop (past the cold-start ROM-config/palette-init code
/// `tests/boot.rs` exercises separately), so the machine under test has
/// real, varied register/GIME/PIA state by the time it's serialized.
const WARMUP_STEPS: u32 = 200_000;

/// Number of instructions to run in lockstep after restore, comparing full
/// CPU state after every single one — enough to run well past the restore
/// point without ballooning test time.
const LOCKSTEP_STEPS: u32 = 50_000;

#[test]
fn snapshot_round_trip_continues_trace_identically() {
    let mut original = boot_machine();
    for _ in 0..WARMUP_STEPS {
        original.step();
    }

    let mut bytes = Vec::new();
    ciborium::into_writer(&original, &mut bytes).expect("serialize warmed-up machine");

    let mut restored: Machine =
        ciborium::from_reader(bytes.as_slice()).expect("deserialize snapshot");

    // No ROM leakage: `SystemBus::rom` is `#[serde(skip)]` (COPYRIGHTED), so
    // the deserialized-but-not-yet-reattached machine must come back with no
    // ROM image at all.
    //
    // Deviation from the spec's literal check (a byte-string search for the
    // ROM's first 64 bytes anywhere in the CBOR stream): by
    // `WARMUP_STEPS` (200_000 instructions in), stock Super Extended Color
    // BASIC's cold-start has already copied its own 32K ROM image into the
    // top of physical RAM (verified: present at physical offset $78000 on
    // the default 512K config) — legitimate emulated machine state that
    // `bus.ram` (not skipped) must faithfully round-trip. A byte-search
    // finds that RAM copy and reports a false leak. Asserting the skipped
    // `rom` field itself comes back empty is the precise version of the same
    // check and isn't confounded by RAM that happens to mirror ROM content.
    assert!(
        restored.bus.rom.is_empty(),
        "deserialized snapshot must not carry ROM bytes through the skipped `rom` field"
    );

    restored.bus.reattach_rom(load_rom());
    restored.after_restore();

    // Lockstep both machines and compare full CPU state after every step.
    for i in 0..LOCKSTEP_STEPS {
        original.step();
        restored.step();
        assert_eq!(
            CpuSnapshot::of(&original.cpu),
            CpuSnapshot::of(&restored.cpu),
            "CPU state diverged at lockstep instruction {i}"
        );
    }

    assert_eq!(
        original.bus.ram, restored.bus.ram,
        "RAM contents diverged after {LOCKSTEP_STEPS} lockstep instructions"
    );
}

/// A minimal out-of-crate [`Cartridge`] implementation, standing in for a
/// test double the way integration tests elsewhere in this crate use one
/// (`cart.rs`'s module doc: `Cart::Custom` exists specifically for these).
struct TestDoubleCart;

impl Cartridge for TestDoubleCart {
    fn read(&mut self, _addr: u16) -> u8 {
        0xFF
    }
    fn write(&mut self, _addr: u16, _val: u8) {}
}

#[test]
fn custom_cart_fails_to_serialize_with_an_error_not_a_panic() {
    let mut m = boot_machine();
    m.insert_cartridge(Cart::custom(TestDoubleCart));

    let mut bytes = Vec::new();
    let result = ciborium::into_writer(&m, &mut bytes);
    assert!(
        result.is_err(),
        "serializing a machine with a Cart::Custom test double must fail, not succeed"
    );
}

