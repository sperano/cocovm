use coco_core::{MachineConfig, MachineVariant, MemorySize, MonitorType};

use super::*;
use crate::machine_def::{CartridgeDTO, PeripheralsDTO};

/// [`EMPTY_DESCRIPTION`] spells out the defaults an empty machine starts
/// with; a change to them must update it.
#[test]
fn empty_description_matches_the_default_machine() {
    let config = MachineConfig::default();
    assert_eq!(config.variant, MachineVariant::Coco3);
    assert_eq!(config.memory, MemorySize::K512);
    assert_eq!(config.monitor, Some(MonitorType::RGB));
    assert_eq!(PeripheralsDTO::default().cartridge, CartridgeDTO::None);
    assert!(EMPTY_DESCRIPTION.starts_with("CoCo 3 with 512K and an RGB monitor, no cartridge"));
}
