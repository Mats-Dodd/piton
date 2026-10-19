//! Stateright explores every state the real session can reach within small
//! bounds.

#![cfg(not(piton_planted_bug))]

use piton_model::{Checked, WorldConfig};
use stateright::{Checker as _, Model as _};

#[test]
fn every_reachable_state_keeps_the_invariants() {
    let checker = Checked(WorldConfig::exhaustive())
        .checker()
        .threads(std::thread::available_parallelism().map_or(1, usize::from))
        .spawn_bfs()
        .join();
    checker.assert_properties();
    assert!(
        checker.unique_state_count() > 900_000,
        "bounds too small to mean much: {} states",
        checker.unique_state_count()
    );
}
