//! Per-variant defaults shared across the manager's "New…" dialog
//! (`new_vm.rs`), machine-definition loading (`machine_def.rs`), the manager
//! list row and window title (`manager::list`, `save_state::restore`), and
//! the lifecycle module's freshly-created-machine name
//! (`manager::lifecycle`) — small enough, and used widely enough outside any
//! one of those modules, to live on their own rather than inside any single
//! caller.

use coco_core::{MachineVariant, MemorySize, VDGVariant};

/// Short label for the window title.
pub(crate) const fn machine_label(variant: MachineVariant) -> &'static str {
    match variant {
        MachineVariant::Coco1 => "CoCo 1",
        MachineVariant::Coco2 => "CoCo 2",
        MachineVariant::Coco3 => "CoCo 3",
    }
}

/// Per-variant default RAM size, used by the VM manager's "New…" dialog
///.
pub(crate) fn default_ram(variant: MachineVariant) -> MemorySize {
    match variant {
        MachineVariant::Coco3 => MemorySize::K512,
        MachineVariant::Coco1 | MachineVariant::Coco2 => MemorySize::K64,
    }
}

/// Per-variant default VDG chip when no explicit choice is made: the T1
/// (CoCo 2B) on a CoCo 2, the plain MC6847 on a CoCo 1 (the only choice
/// `MachineConfig::validate` accepts there), and `None` on a CoCo 3, which
/// has no VDG at all. Shared by `new_vm.rs`'s `constrain` and
/// `machine_def.rs`'s `to_machine_config`.
pub(crate) const fn default_vdg(variant: MachineVariant) -> Option<VDGVariant> {
    match variant {
        MachineVariant::Coco2 => Some(VDGVariant::MC6847T1),
        MachineVariant::Coco1 => Some(VDGVariant::MC6847),
        MachineVariant::Coco3 => None,
    }
}

#[cfg(test)]
#[path = "defaults_test.rs"]
mod tests;
