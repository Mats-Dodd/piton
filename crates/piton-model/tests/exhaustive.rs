//! Stateright explores every state the real session can reach within small
//! bounds: once with crashes and late appends, once with races between
//! completions, timers and stale deliveries.

#![cfg(not(piton_planted_bug))]

use piton_model::{Checked, WorldConfig};
use stateright::{Checker as _, Model as _};

fn explore(config: WorldConfig, at_least: usize) {
    let checker = Checked(config)
        .checker()
        .threads(std::thread::available_parallelism().map_or(1, usize::from))
        .spawn_bfs()
        .join();
    checker.assert_properties();
    assert!(
        checker.unique_state_count() > at_least,
        "bounds too small to mean much: {} states",
        checker.unique_state_count()
    );
}

#[test]
fn every_state_with_crashes_keeps_the_invariants() {
    explore(WorldConfig::crashes(), 900_000);
}

#[test]
fn every_state_with_races_keeps_the_invariants() {
    explore(WorldConfig::races(), 150_000);
}
