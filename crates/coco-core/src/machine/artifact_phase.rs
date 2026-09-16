//! Reset-time NTSC artifact-phase selection for CoCo 1/2 machines.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use serde::{Deserialize, Serialize};

use crate::config::MachineVariant;
use crate::video::RG6ArtifactPhase;

const DEFAULT_RANDOM_STATE: u64 = 0x6a09_e667_f3bc_c909;
const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;
const SPLITMIX_MULTIPLIER_1: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX_MULTIPLIER_2: u64 = 0x94d0_49bb_1331_11eb;
const ENTROPY_DOMAIN: u64 = 0x4e54_5343_5247_3600;

#[derive(Serialize, Deserialize)]
pub(super) struct ArtifactPhaseState {
    selected: RG6ArtifactPhase,
    random_state: u64,
}

impl ArtifactPhaseState {
    pub(super) fn new(seed: u64, variant: MachineVariant) -> Self {
        let mut state = Self {
            selected: RG6ArtifactPhase::Standard,
            random_state: seed,
        };
        state.select_for_reset(variant);
        state
    }

    pub(super) fn selected(&self) -> RG6ArtifactPhase {
        self.selected
    }

    pub(super) fn select_for_reset(&mut self, variant: MachineVariant) {
        if matches!(variant, MachineVariant::Coco1 | MachineVariant::Coco2) {
            self.selected = if self.next_random() & 1 == 0 {
                RG6ArtifactPhase::Standard
            } else {
                RG6ArtifactPhase::Reverse
            };
        }
    }

    fn next_random(&mut self) -> u64 {
        self.random_state = self.random_state.wrapping_add(SPLITMIX_INCREMENT);
        let mut value = self.random_state;
        value = (value ^ (value >> 30)).wrapping_mul(SPLITMIX_MULTIPLIER_1);
        value = (value ^ (value >> 27)).wrapping_mul(SPLITMIX_MULTIPLIER_2);
        value ^ (value >> 31)
    }
}

impl Default for ArtifactPhaseState {
    fn default() -> Self {
        Self {
            selected: RG6ArtifactPhase::Standard,
            random_state: DEFAULT_RANDOM_STATE,
        }
    }
}

pub(super) fn fresh_seed() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(ENTROPY_DOMAIN);
    hasher.finish()
}
