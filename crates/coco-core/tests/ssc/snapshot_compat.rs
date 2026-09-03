//! Snapshot evolution rule 2 (`crate::snapshot`): a cartridge payload saved
//! before the speech fields existed must still deserialize, with the chip in
//! reset and ready for its ROM to be reattached.

use coco_core::ssc::SoundSpeechCartridge;
use coco_core::{MachineVariant, MemorySize};
use mc6809::Bus;

use super::common::{BLANK_SPEECH_ROM, FF7E, bus_with_ssc, ssc_without_speech};

/// Serialized field names added by the SP0256 work.
const SPEECH_FIELDS: [&str; 2] = ["sp0256", "speech"];

#[test]
fn a_pre_speech_snapshot_deserializes_with_the_chip_in_reset() {
    let saved = ssc_without_speech();
    let mut value = ciborium::Value::serialized(&saved).expect("serializes");
    let ciborium::Value::Map(entries) = &mut value else {
        panic!("cartridge serializes as a map");
    };
    let before = entries.len();
    entries.retain(
        |(key, _)| !matches!(key, ciborium::Value::Text(t) if SPEECH_FIELDS.contains(&t.as_str())),
    );
    assert_eq!(
        entries.len(),
        before - SPEECH_FIELDS.len(),
        "both fields were present"
    );

    let mut restored: SoundSpeechCartridge = value.deserialized().expect("pre-field payload loads");
    restored
        .reattach_speech_rom(&BLANK_SPEECH_ROM)
        .expect("restore re-supplies the ROM");

    let mut b = bus_with_ssc(MachineVariant::Coco3, MemorySize::K512);
    b.cart = restored.into();
    assert_eq!(
        b.read(FF7E) & 0x40,
        0x40,
        "bit 6 idle: the chip is in reset"
    );
}
