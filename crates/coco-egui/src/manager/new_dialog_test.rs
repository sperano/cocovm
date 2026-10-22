use coco_core::{MachineConfig, MachineVariant, MemorySize, MonitorType};

use super::*;
use crate::machine_def::{CartridgeDTO, DriveWireDTO, MachineDef, MediaDTO};

/// [`EMPTY_DESCRIPTION`] spells out the defaults an empty machine starts
/// with; a change to them, or to the text, must update this test.
#[test]
fn empty_description_matches_the_default_machine() {
    let config = MachineConfig::default();
    assert_eq!(config.variant, MachineVariant::Coco3);
    assert_eq!(config.memory, MemorySize::K512);
    assert_eq!(config.monitor, Some(MonitorType::RGB));
    let def = MachineDef::from_config(String::new(), None, &config);
    assert_eq!(def.peripherals.cartridge, CartridgeDTO::None);
    assert_eq!(def.media, MediaDTO::default());
    assert_eq!(def.drivewire, DriveWireDTO::default());
    assert_eq!(
        EMPTY_DESCRIPTION,
        "CoCo 3 with 512K and an RGB monitor, no cartridge, and no disks, to set up yourself"
    );
}
