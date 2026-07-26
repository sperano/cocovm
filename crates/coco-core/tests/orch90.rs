//! Orchestra-90/CC coverage: `$FF7A`/`$FF7B` DAC latch decode through the
//! widened `$FF60-$FF7E` cartridge I/O window, the mux-independent mono mix
//! (Tier 1, `docs/plan-orchestra-90.md`), MPI slot behaviour, and the
//! autostart FIRQ boot path against the real `roms/coco3.rom`.

use std::path::PathBuf;

use coco_core::cart::{Cartridge, MultiPak};
use coco_core::orch90::{LEFT_DAC_REG, Orch90, RIGHT_DAC_REG};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

/// An Orch-90 with an all-zero (harmless) 8K ROM image, for bus-level tests
/// that never execute cart code.
fn orch90() -> Orch90 {
    Orch90::from_rom_bytes(&[0u8; 8 * 1024]).unwrap()
}

fn bus_with_orch90() -> SystemBus {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.cart = orch90().into();
    b
}

// ---- Latch decode through the bus ---------------------------------------------

#[test]
fn dac_writes_latch_independently_through_the_bus() {
    let mut b = bus_with_orch90();
    b.write(LEFT_DAC_REG, 0x40);
    b.write(RIGHT_DAC_REG, 0xC0);
    let o = b.cart.as_orch90().expect("an Orch-90 is inserted");
    assert_eq!((o.left(), o.right()), (0x40, 0xC0));

    // Rewriting one channel must not disturb the other.
    b.write(LEFT_DAC_REG, 0xFF);
    let o = b.cart.as_orch90().unwrap();
    assert_eq!((o.left(), o.right()), (0xFF, 0xC0));
}

#[test]
fn dac_registers_are_write_only() {
    // No read path back from the 74LS374 latches: reads float (open bus),
    // regardless of what was latched.
    let mut b = bus_with_orch90();
    b.write(LEFT_DAC_REG, 0x12);
    b.write(RIGHT_DAC_REG, 0x34);
    assert_eq!(b.read(LEFT_DAC_REG), 0xFF);
    assert_eq!(b.read(RIGHT_DAC_REG), 0xFF);
}

#[test]
fn neighboring_expansion_addresses_do_not_alias_the_latches() {
    // MAME installs exact single-byte handlers at $FF7A/$FF7B — no mirrors.
    let mut b = bus_with_orch90();
    b.write(LEFT_DAC_REG - 1, 0x55); // $FF79
    b.write(RIGHT_DAC_REG + 1, 0x66); // $FF7C
    let o = b.cart.as_orch90().unwrap();
    assert_eq!((o.left(), o.right()), (0, 0));
}

// ---- Tier 1 mono mix ------------------------------------------------------------

#[test]
fn cart_audio_reaches_the_speaker_regardless_of_mux_state() {
    // The Orch-90 drives its own RCA outputs, not the CoCo's SND pin, so the
    // SNDEN/SEL mux must never gate it (MAME coco_orch90.cpp routes the DACs
    // to their own speaker, ignoring SOUND_ENABLE). Fresh bus: SNDEN low,
    // SEL=00 — the internal DAC path is silent either way.
    let mut b = bus_with_orch90();
    assert_eq!(b.sound_probe(PROBE_DT), [0.0; 2], "latches power on at 0: silent");

    b.write(LEFT_DAC_REG, 0xFF);
    b.write(RIGHT_DAC_REG, 0xFF);
    let [full_l, full_r] = b.sound_probe(PROBE_DT);
    assert!(full_l > 0.5, "full-scale L with SNDEN low: {full_l}");
    assert_eq!(full_l, full_r, "equal latches are centred");

    // True stereo: zeroing one DAC silences ONLY that channel (hard pan) —
    // the plan's stereo acceptance test.
    b.write(RIGHT_DAC_REG, 0x00);
    let [l, r] = b.sound_probe(PROBE_DT);
    assert_eq!(l, full_l, "left channel unchanged");
    assert_eq!(r, 0.0, "right channel silent");
}

#[test]
fn sound_levels_reports_the_two_latches_as_a_stereo_pair() {
    let mut o = orch90();
    o.write(LEFT_DAC_REG, 0xFF);
    assert_eq!(o.sound_levels(), (1.0, 0.0));
    o.write(RIGHT_DAC_REG, 0xFF);
    assert_eq!(o.sound_levels(), (1.0, 1.0));
}

// ---- Through the MPI -------------------------------------------------------------

#[test]
fn mpi_dac_writes_ignore_the_slot_select_and_audio_sums() {
    const ORCH_SLOT: usize = 1;
    const OTHER_SLOT: usize = 3;
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    let mut mp = MultiPak::new(ORCH_SLOT);
    mp.insert(ORCH_SLOT, orch90());
    b.cart = mp.into();

    b.write(LEFT_DAC_REG, 0xFF);
    b.write(RIGHT_DAC_REG, 0xFF);
    assert_eq!(
        b.cart.sound_levels(),
        (1.0, 1.0),
        "write reached the pak's DAC latches"
    );

    // Analog SND is common to all slots (only SCS*/CTS*/CART* are switched):
    // deselecting the slot leaves the held latches on the wire.
    b.cart.as_multipak().unwrap().set_switch(OTHER_SLOT);
    assert_eq!(b.cart.sound_levels(), (1.0, 1.0), "held level still on the wire");
    // $FF7A/$FF7B sit in the $FF60-$FF7E extension window, which the MPI
    // does not switch either — the pak full-decodes the address bus, so a
    // write lands regardless of the slot select.
    b.write(LEFT_DAC_REG, 0x00);
    let o = b.cart.as_orch90().unwrap();
    assert_eq!(o.left(), 0x00, "write lands despite the slot select");
    assert_eq!(b.cart.sound_levels(), (0.0, 1.0), "only the right DAC still held");
}

// ---- Real-ROM autostart integration ----------------------------------------------

fn load_coco3_rom() -> Box<[u8]> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .into_boxed_slice()
}

/// Values the synthetic pak program latches into each channel.
/// Generator step for `sound_probe` (the Orch-90 has no crystal generators —
/// its DACs are latches, so this value is inert here).
const PROBE_DT: f64 = 1.0 / 62_866.0;

const LEFT_MARKER: u8 = 0xA5;
const RIGHT_MARKER: u8 = 0x5A;

/// An 8K pak image whose entry code at $C000 latches [`LEFT_MARKER`]/
/// [`RIGHT_MARKER`] into the DACs and loops:
/// `LDA #$A5 ; STA $FF7A ; LDA #$5A ; STA $FF7B ; BRA *`.
fn marker_orch90() -> Orch90 {
    let program = [
        0x86, LEFT_MARKER, 0xB7, 0xFF, 0x7A, // LDA #imm ; STA $FF7A
        0x86, RIGHT_MARKER, 0xB7, 0xFF, 0x7B, // LDA #imm ; STA $FF7B
        0x20, 0xFE, // BRA *
    ];
    let mut image = vec![0u8; 8 * 1024];
    image[..program.len()].copy_from_slice(&program);
    Orch90::from_rom_bytes(&image).unwrap()
}

#[test]
fn orch90_autostarts_and_its_cart_code_drives_the_dacs() {
    // Bounded so a regression (CART*->CB1 never pulses, FIRQ never fires,
    // or the $FF7A/$FF7B decode breaks) fails instead of hanging.
    const MAX_FIELDS: usize = 400;
    let mut m = Machine::new(MachineConfig::default(), load_coco3_rom());
    m.insert_cartridge(marker_orch90());
    m.reset();

    let mut latched = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        let o = m.bus.cart.as_orch90().unwrap();
        if (o.left(), o.right()) == (LEFT_MARKER, RIGHT_MARKER) {
            latched = true;
            break;
        }
    }
    assert!(
        latched,
        "cart code at $C000 never latched the DACs: CART*->FIRQ autostart or \
         $FF7A/$FF7B decode is broken"
    );
    // And the mono fold-down of those latches is audible in the field's
    // sample stream.
    m.run_field();
    let audible = m.take_audio().any(|s| s[0] > 0.0 || s[1] > 0.0);
    assert!(audible, "latched DACs produced no audio samples");
}
