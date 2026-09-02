//! Snapshot evolution rule 2 (`crate::snapshot`): a cartridge payload saved
//! before the speech fields existed must still restore, chip-less.

use coco_core::ssc::SoundSpeechCartridge;
use coco_core::{MachineVariant, MemorySize};
use mc6809::Bus;

use super::common::{FF7E, bus_with_ssc};

/// Serialized field names added by the SP0256 work.
const SPEECH_FIELDS: [&str; 2] = ["sp0256", "speech"];

#[test]
fn a_pre_speech_snapshot_restores_chip_less() {
    let saved = SoundSpeechCartridge::new();
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

    let restored: SoundSpeechCartridge = value.deserialized().expect("pre-field payload loads");
    assert!(!restored.has_speech_chip());

    let mut b = bus_with_ssc(MachineVariant::Coco3, MemorySize::K512);
    b.cart = restored.into();
    assert_eq!(b.read(FF7E) & 0x40, 0x40, "bit 6 idle, as before the field");
}
