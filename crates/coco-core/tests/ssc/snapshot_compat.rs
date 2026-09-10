//! Missing firmware fields deserialize with defaults. A payload saved
//! mid-speech resumes once both ROMs are reattached, and the engine keys
//! the two images by [`CartROMRole`].

use std::path::PathBuf;

use coco_core::snapshot::{
    self, CartROMRole, CartROMSource, MediaRef, MediaRefs, MediaSources, SlotROMRef, SnapshotError,
};
use coco_core::ssc::{SoundSpeechCartridge, cmd, terminator};
use coco_core::{Machine, MachineConfig, MachineVariant, MemorySize};
use mc6809::Bus;
use test_assets::rom::COCO3;

use super::common::{
    BLANK_FIRMWARE, BLANK_SPEECH_ROM, FF7E, SPEECH_MAX_PUMPS, SPEECH_READY, blank_ssc, bus_with,
    pump, send, skip, try_coco3_bus_with_ssc, wait_for_speech,
};

/// Serialized microcontroller, board, and execution-budget field names.
const FIRMWARE_FIELDS: [&str; 3] = ["tms", "board", "tms_budget"];

#[test]
fn missing_firmware_fields_deserialize_with_reset_pending() {
    let saved = blank_ssc();
    let mut value = ciborium::Value::serialized(&saved).expect("serializes");
    let ciborium::Value::Map(entries) = &mut value else {
        panic!("cartridge serializes as a map");
    };
    let before = entries.len();
    entries.retain(
        |(key, _)| !matches!(key, ciborium::Value::Text(t) if FIRMWARE_FIELDS.contains(&t.as_str())),
    );
    assert_eq!(entries.len(), before - FIRMWARE_FIELDS.len());

    let mut restored: SoundSpeechCartridge =
        value.deserialized().expect("missing fields use defaults");
    assert!(restored.firmware().pending_reset());
    restored.reattach_firmware_rom(&BLANK_FIRMWARE).unwrap();
    restored.reattach_speech_rom(&BLANK_SPEECH_ROM).unwrap();

    let mut b = bus_with(restored, MachineVariant::Coco3, MemorySize::K512);
    b.cart.tick(100);
    assert!(
        !b.cart.as_ssc().unwrap().firmware().pending_reset(),
        "booted on the first tick"
    );
    assert_eq!(b.read(FF7E) & SPEECH_READY, SPEECH_READY);
}

#[test]
fn a_mid_speech_snapshot_resumes_after_both_roms_are_reattached() {
    let Some(mut b) = try_coco3_bus_with_ssc() else {
        return skip("a_mid_speech_snapshot_resumes_after_both_roms_are_reattached");
    };
    const OY: u8 = 5;
    const PA5: u8 = 4;
    send(&mut b, cmd::LOAD_ALLOPHONE_INDIVIDUAL_START);
    for a in [OY, OY, OY, PA5] {
        send(&mut b, a);
    }
    send(&mut b, terminator::SOUND);
    send(&mut b, cmd::EXEC_ALLOPHONE_INDIVIDUAL_START);
    wait_for_speech(&mut b);
    pump(&mut b, 500);
    assert_eq!(b.read(FF7E) & SPEECH_READY, 0, "mid-speech");

    let cycles_before = b.cart.as_ssc().unwrap().firmware().cycles;
    let value = ciborium::Value::serialized(b.cart.as_ssc().unwrap()).expect("serializes");
    let mut restored: SoundSpeechCartridge = value.deserialized().expect("round trip");
    let firmware = std::fs::read(test_assets::rom(test_assets::rom::SSC_TMS7040)).unwrap();
    let speech = std::fs::read(test_assets::rom(test_assets::rom::SP0256_AL2)).unwrap();
    restored.reattach_firmware_rom(&firmware).unwrap();
    restored.reattach_speech_rom(&speech).unwrap();
    assert_eq!(restored.firmware().cycles, cycles_before);

    let mut b2 = bus_with(restored, MachineVariant::Coco3, MemorySize::K512);
    assert_eq!(b2.read(FF7E) & SPEECH_READY, 0, "still mid-speech");
    let mut pumps = 0;
    while b2.read(FF7E) & SPEECH_READY == 0 {
        pump(&mut b2, 1);
        pumps += 1;
        assert!(
            pumps < SPEECH_MAX_PUMPS,
            "speech never finished after the round trip"
        );
    }
    assert!(pumps > 0);
}

/// A machine with a blank-ROM cartridge in the port, saved with both images
/// recorded, plus the sources restore needs. The images differ in size, so a
/// restore that mixed the roles up would fail as [`SnapshotError::MediaShape`].
fn saved_with_both_images() -> (snapshot::SnapshotPayload, Box<[u8]>) {
    let rom_path = test_assets::rom(COCO3);
    let rom: Box<[u8]> = std::fs::read(&rom_path)
        .expect("coco3.rom")
        .into_boxed_slice();
    let mut machine = Machine::new(MachineConfig::default(), rom.clone());
    machine.insert_cartridge(blank_ssc());
    let slot_ref = |role, name: &str, bytes: &[u8]| SlotROMRef {
        mpi_slot: None,
        role,
        rom: MediaRef {
            path: PathBuf::from(name),
            sha256: snapshot::sha256_hex(bytes),
        },
    };
    let media = MediaRefs {
        system_rom: Some(MediaRef {
            path: rom_path,
            sha256: snapshot::sha256_hex(&rom),
        }),
        cart_roms: vec![
            slot_ref(CartROMRole::Primary, "sp0256-al2.rom", &BLANK_SPEECH_ROM),
            slot_ref(CartROMRole::SSCFirmware, "ssc-tms7040.rom", &BLANK_FIRMWARE),
        ],
        ..MediaRefs::default()
    };
    let bytes = snapshot::save(&machine, &media).expect("save");
    (snapshot::load(&bytes).expect("load"), rom)
}

#[test]
fn restore_reattaches_each_ssc_image_by_role() {
    let (payload, rom) = saved_with_both_images();
    let sources = MediaSources {
        system_rom: Some(rom),
        cart_roms: vec![
            CartROMSource::primary(None, BLANK_SPEECH_ROM.to_vec()),
            CartROMSource {
                mpi_slot: None,
                role: CartROMRole::SSCFirmware,
                bytes: BLANK_FIRMWARE.to_vec(),
            },
        ],
        ..MediaSources::default()
    };
    let Ok(mut restored) = snapshot::restore(payload, sources) else {
        panic!("both images reattach");
    };
    assert!(restored.machine.bus.cart.as_ssc().is_some());
}

#[test]
fn restore_without_the_firmware_source_names_it_as_missing() {
    let (payload, rom) = saved_with_both_images();
    let sources = MediaSources {
        system_rom: Some(rom),
        cart_roms: vec![CartROMSource::primary(None, BLANK_SPEECH_ROM.to_vec())],
        ..MediaSources::default()
    };
    match snapshot::restore(payload, sources) {
        Err(SnapshotError::MissingMedia { descriptions }) => assert!(
            descriptions
                .iter()
                .any(|d| d.contains("TMS7040 firmware") && d.contains("ssc-tms7040.rom")),
            "{descriptions:?}"
        ),
        Err(other) => panic!("expected MissingMedia, got {other:?}"),
        Ok(_) => panic!("restore must not succeed without the firmware"),
    }
}
