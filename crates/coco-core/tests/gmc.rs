//! Games Master Cartridge coverage: the `$FF40` 16K bank latch (the
//! RoboCop/Predator banked-pak circuit), the `$FF41` SN76489A port routed
//! through the bus, the unconditional speaker mix, and MPI behaviour —
//! registers follow the SCS-selected slot while audio plays from any slot
//! (; MAME `coco_gmc.cpp`,
//! `coco_pak.cpp`).

use coco_core::cart::{
    BANKED_PAK_MAX_LEN, BANKED_PAK_WINDOW_LEN, BankedPakError, BankedROMPak, Cartridge,
    GamesMasterCartridge, MultiPak,
};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize, SystemBus};
use mc6809::Bus;

/// `$FF40`: the bank latch.
const BANK_REG: u16 = 0xFF40;
/// `$FF41`: the SN76489A command port.
const PSG_REG: u16 = 0xFF41;
/// `$FF7F`: the MPI select register.
const MPI_SELECT: u16 = 0xFF7F;

/// One scanline's wall time at the default NTSC timing — the cadence
/// `Machine::flush_line_audio` drives `generator_sample` at (one grid slot).
const LINE_DT: f64 = 1.0 / (262.0 * 60.0);

/// A banked image with `banks` 16K pages, where every byte of page `n` is
/// `marker(n)`.
fn banked_image(banks: usize) -> Vec<u8> {
    let mut image = vec![0u8; banks * BANKED_PAK_WINDOW_LEN];
    for (n, page) in image.chunks_mut(BANKED_PAK_WINDOW_LEN).enumerate() {
        page.fill(marker(n));
    }
    image
}

fn marker(bank: usize) -> u8 {
    0xB0 | bank as u8
}

/// The four attenuation-code-15 writes that silence every PSG channel.
const PSG_MUTE_ALL: [u8; 4] = [0x9F, 0xBF, 0xDF, 0xFF];

// ---- BankedROMPak -------------------------------------------------------------

#[test]
fn rejects_empty_and_oversized_images() {
    assert_eq!(
        BankedROMPak::from_bytes(&[], false).unwrap_err(),
        BankedPakError::Empty
    );
    let bytes = vec![0u8; BANKED_PAK_MAX_LEN + 1];
    assert_eq!(
        BankedROMPak::from_bytes(&bytes, false).unwrap_err(),
        BankedPakError::TooLarge {
            len: BANKED_PAK_MAX_LEN + 1
        }
    );
}

#[test]
fn bank_latch_pages_the_16k_window() {
    let mut pak = BankedROMPak::from_bytes(&banked_image(8), false).unwrap();
    assert_eq!(pak.rom_read(0xC000), marker(0), "power-on bank is 0");
    for bank in 0..8 {
        pak.write(BANK_REG, bank as u8);
        assert_eq!(pak.rom_read(0xC000), marker(bank));
        assert_eq!(pak.rom_read(0xFDFF), marker(bank), "window top");
    }
}

#[test]
fn window_mirrors_across_both_16k_halves_of_the_external_map() {
    // Under INIT0's 32K-external map the same 16K bank shows at $8000 and
    // $C000 (the GIME half-swap flips a bit the 16K window mask discards).
    let mut pak = BankedROMPak::from_bytes(&banked_image(8), false).unwrap();
    pak.write(BANK_REG, 5);
    assert_eq!(pak.rom_read(0x8000), marker(5));
    assert_eq!(pak.rom_read(0xA000), marker(5));
    assert_eq!(pak.rom_read(0xC000), marker(5));
}

#[test]
fn bank_latch_wraps_modulo_128k_and_undersized_images_mirror() {
    // A 4-bank (64K) image mirror-fills to 128K, so banks 4-7 repeat banks
    // 0-3; the raw latch byte itself wraps modulo the 128K space (MAME
    // `(m_pos * 0x4000) % m_eprom->bytes()`), so bank 9 lands on bank 1.
    let mut pak = BankedROMPak::from_bytes(&banked_image(4), false).unwrap();
    pak.write(BANK_REG, 6);
    assert_eq!(pak.rom_read(0xC000), marker(2), "mirror-filled upper half");
    pak.write(BANK_REG, 9);
    assert_eq!(pak.rom_read(0xC000), marker(1), "latch wraps modulo 128K");
}

#[test]
fn reset_returns_to_bank_zero() {
    let mut pak = BankedROMPak::from_bytes(&banked_image(8), false).unwrap();
    pak.write(BANK_REG, 3);
    assert_eq!(pak.rom_read(0xC000), marker(3));
    pak.reset();
    assert_eq!(pak.rom_read(0xC000), marker(0));
}

// ---- GamesMasterCartridge through the bus --------------------------------------

fn bus_with_gmc() -> SystemBus {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    b.cart = GamesMasterCartridge::from_bytes(&banked_image(8), true)
        .unwrap()
        .into();
    b
}

#[test]
fn ff40_switches_the_gmc_rom_bank_through_the_bus() {
    let mut b = bus_with_gmc();
    b.write(BANK_REG, 4);
    // The external window needs INIT0's ROM-map bits pointing at the cart;
    // read via the cart directly to keep the test on the latch itself.
    assert_eq!(b.cart.rom_read(0xC000), marker(4));
}

#[test]
fn ff41_reaches_the_psg_and_the_speaker_mix() {
    let mut b = bus_with_gmc();
    // Power-on state hums (max volume everywhere — MAME-verified); let the
    // tone counters run into their audible phase first.
    let mut heard = false;
    for _ in 0..262 {
        if b.sound_probe(LINE_DT)[0] > 0.0 {
            heard = true;
        }
    }
    assert!(heard, "power-on PSG hum must reach the speaker mix");

    // Silencing every channel through the bus write path kills it.
    for cmd in PSG_MUTE_ALL {
        b.write(PSG_REG, cmd);
    }
    for _ in 0..262 {
        assert_eq!(b.sound_probe(LINE_DT)[0], 0.0, "muted PSG must be silent");
    }
}

#[test]
fn machine_mixes_gmc_audio_into_the_field_samples() {
    let mut m = Machine::new(
        MachineConfig::default(),
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    m.bus.write(0x0000, 0x20); // BRA *
    m.bus.write(0x0001, 0xFE);
    m.insert_cartridge(GamesMasterCartridge::from_bytes(&banked_image(8), false).unwrap());
    m.run_field();
    m.run_field();
    assert!(
        m.take_audio().any(|s| s[0] > 0.0),
        "the un-initialized PSG's hum must show up in the field's samples"
    );
}

// ---- End-to-end: cart code programs the PSG through the real boot path ---------

/// 6809 code a GMC game would run, assembled by hand (opcodes: `ORCC` #imm
/// `1A`, `LDA` #imm `86`, `STA` ext `B7`, `BRA` `20`): mask interrupts, then
/// program tone 0 to period $01E at max volume, mute the other three
/// channels, and loop. Runs at `$C000` via the autostart FIRQ path.
#[rustfmt::skip]
const PSG_PLAYER: [u8; 34] = [
    0x1A, 0x50,             // ORCC #$50      (mask IRQ/FIRQ)
    0x86, 0x8E, 0xB7, 0xFF, 0x41, // tone 0 latch: period low nibble E
    0x86, 0x01, 0xB7, 0xFF, 0x41, // data byte: period = $01E (30 ticks)
    0x86, 0x90, 0xB7, 0xFF, 0x41, // tone 0 attenuation 0 (max)
    0x86, 0xBF, 0xB7, 0xFF, 0x41, // tone 1 mute
    0x86, 0xDF, 0xB7, 0xFF, 0x41, // tone 2 mute
    0x86, 0xFF, 0xB7, 0xFF, 0x41, // noise mute
    0x20, 0xFE,             // BRA *
];

/// Speaker level of the lone max-volume tone 0 through the mix:
/// channel full scale (0.25) × the bus's cartridge gain (0.75).
const LONE_TONE_LEVEL: f32 = 0.25 * 0.75;

#[test]
fn autostarted_cart_code_plays_a_tone_through_the_speaker() {
    let rom_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/coco3.rom");
    let rom = std::fs::read(&rom_path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", rom_path.display()))
        .into_boxed_slice();
    let mut image = vec![0u8; BANKED_PAK_WINDOW_LEN];
    image[..PSG_PLAYER.len()].copy_from_slice(&PSG_PLAYER);

    let mut m = Machine::new(MachineConfig::default(), rom);
    m.insert_cartridge(GamesMasterCartridge::from_bytes(&image, true).unwrap());
    m.reset();

    // Boot until the cart code has silenced the power-on hum (bounded so a
    // dead autostart path fails instead of hanging), then inspect one field.
    const MAX_FIELDS: usize = 400;
    let mut programmed = false;
    for _ in 0..MAX_FIELDS {
        m.run_field();
        let samples: Vec<f32> = m.take_audio().map(|s| s[0]).collect();
        if samples.iter().all(|&s| s <= LONE_TONE_LEVEL) {
            programmed = true;
            let loud = samples
                .iter()
                .filter(|&&s| s > LONE_TONE_LEVEL * 0.5)
                .count();
            let quiet = samples
                .iter()
                .filter(|&&s| s < LONE_TONE_LEVEL * 0.1)
                .count();
            assert!(
                loud > 10 && quiet > 10,
                "expected the programmed ~4.2 kHz square wave to oscillate \
                 (loud={loud} quiet={quiet} of {})",
                samples.len()
            );
            break;
        }
    }
    assert!(
        programmed,
        "cart code never reprogrammed the PSG: the CART*->FIRQ->$C000->$FF41 path is broken"
    );
}

// ---- GMC in a Multi-Pak slot ----------------------------------------------------

#[test]
fn mpi_routes_psg_writes_to_the_selected_slot_only_but_audio_from_any() {
    let mut b = SystemBus::new(
        MachineVariant::Coco3,
        MemorySize::K512,
        vec![0u8; 32 * 1024].into_boxed_slice(),
    );
    let mut mp = MultiPak::new(0);
    mp.insert(
        1,
        GamesMasterCartridge::from_bytes(&banked_image(8), false).unwrap(),
    );
    b.cart = mp.into();

    // Slot 1 (the GMC) is not SCS-selected (switch points at slot 0): the
    // mute writes must not reach it, and its hum still mixes — the SND pin
    // is common to all MPI slots.
    for cmd in PSG_MUTE_ALL {
        b.write(PSG_REG, cmd);
    }
    let mut heard = false;
    for _ in 0..262 {
        if b.sound_probe(LINE_DT)[0] > 0.0 {
            heard = true;
        }
    }
    assert!(heard, "unselected slot's PSG must still be audible");

    // Select slot 1 via $FF7F (SCS bits 1-0), mute again: now it lands.
    b.write(MPI_SELECT, 0xCD);
    for cmd in PSG_MUTE_ALL {
        b.write(PSG_REG, cmd);
    }
    for _ in 0..262 {
        assert_eq!(
            b.sound_probe(LINE_DT)[0],
            0.0,
            "selected slot's PSG must mute"
        );
    }
}
