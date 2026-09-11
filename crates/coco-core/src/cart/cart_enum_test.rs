use super::*;

const TEST_ROM: [u8; 1] = [0];
const GMC_SLOT: usize = 1;
const MPI_SWITCH_SLOT: usize = 0;

fn games_master() -> GamesMasterCartridge {
    GamesMasterCartridge::from_bytes(&TEST_ROM, false).expect("create test GMC")
}

#[test]
fn detects_direct_games_master() {
    let cart = Cart::from(games_master());

    assert!(cart.contains_games_master());
}

#[test]
fn detects_games_master_in_multipak() {
    let mut mpi = MultiPak::new(MPI_SWITCH_SLOT);
    mpi.insert(GMC_SLOT, games_master());
    let cart = Cart::from(mpi);

    assert!(cart.contains_games_master());
}

#[test]
fn reports_no_games_master_for_other_carts() {
    assert!(!Cart::default().contains_games_master());
}
