//! Snapshot evolution rule 2 (`crate::snapshot`): a cartridge payload saved
//! before the firmware fields existed must still deserialize, and a payload
//! saved mid-speech resumes once both ROMs are reattached.

use coco_core::ssc::{SoundSpeechCartridge, cmd, terminator};
use coco_core::{MachineVariant, MemorySize};
use mc6809::Bus;

use super::common::{
    BLANK_FIRMWARE, BLANK_SPEECH_ROM, FF7E, SPEECH_READY, blank_ssc, bus_with, pump, send, skip,
    try_coco3_bus_with_ssc,
};

/// Serialized field names added by the firmware work.
const FIRMWARE_FIELDS: [&str; 3] = ["tms", "board", "tms_budget"];

#[test]
fn a_pre_firmware_snapshot_deserializes_with_a_reset_pending() {
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

    let mut restored: SoundSpeechCartridge = value.deserialized().expect("pre-field payload loads");
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
    while b.read(FF7E) & SPEECH_READY != 0 {
        pump(&mut b, 1);
    }
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
        assert!(pumps < 40_000, "speech never finished after the round trip");
    }
    assert!(pumps > 0);
}
