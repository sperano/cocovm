use super::*;
use coco_core::cart::{GamesMasterCartridge, MultiPak, ROMPak};

const TEST_ROM: [u8; 1] = [0];
const GMC_SLOT: usize = 2;
const MPI_SWITCH_SLOT: usize = 0;

fn games_master() -> GamesMasterCartridge {
    GamesMasterCartridge::from_bytes(&TEST_ROM, false).expect("create test GMC")
}

#[test]
fn direct_games_master_disables_becker_enablement() {
    let cart = games_master().into();

    assert!(!becker_toggle_enabled(false, &cart));
}

#[test]
fn multipak_games_master_disables_becker_enablement() {
    let mut mpi = MultiPak::new(MPI_SWITCH_SLOT);
    mpi.insert(GMC_SLOT, games_master());
    let cart = mpi.into();

    assert!(!becker_toggle_enabled(false, &cart));
}

#[test]
fn ordinary_cartridge_permits_becker_enablement() {
    let cart = ROMPak::from_bytes(&TEST_ROM, false)
        .expect("create test ROM pak")
        .into();

    assert!(becker_toggle_enabled(false, &cart));
}

#[test]
fn enabled_becker_can_be_disabled_with_games_master_present() {
    let cart = games_master().into();

    assert!(becker_toggle_enabled(true, &cart));
}
