//! Seeded simulations of the runtime under faults (`just sim`).
//!
//! `PITON_SIM_SEEDS=n` runs seeds `0..n` (256 by default); `PITON_SIM_SEED=n`
//! runs only seed `n`, to reproduce a failure.

#![cfg(all(tokio_unstable, not(piton_planted_shell_bug)))]

use piton_model::to_ndjson;
use piton_sim::{Coverage, Plan, Run, check, run};

fn seeds() -> std::ops::Range<u64> {
    let read = |name| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
    };
    match (read("PITON_SIM_SEED"), read("PITON_SIM_SEEDS")) {
        (Some(seed), _) => seed..seed.saturating_add(1),
        (None, Some(count)) => 0..count,
        (None, None) => 0..256,
    }
}

/// Writes a failing run's journal where it can be checked in as a
/// regression: journals keep reproducing after the code changes; seeds
/// don't.
fn keep(run: &Run) -> String {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/sim-failures");
    let path = format!("{directory}/seed-{}.ndjson", run.seed);
    let written = std::fs::create_dir_all(directory)
        .and_then(|()| std::fs::write(&path, to_ndjson(&run.journal)));
    match written {
        Ok(()) => path,
        Err(error) => format!("(could not write {path}: {error})"),
    }
}

#[test]
fn every_seed_survives_faults_and_crashes() {
    let seeds = seeds();
    let several = seeds.end.saturating_sub(seeds.start) > 1;
    let mut coverage = Coverage::default();
    for seed in seeds.clone() {
        let run = run(seed, Plan::standard());
        let checked = check(&run);
        assert!(
            checked.is_ok(),
            "seed {seed} failed (rerun with PITON_SIM_SEED={seed}): {:?}\njournal: {}",
            checked.err(),
            keep(&run)
        );
        coverage = coverage.add(Coverage::of(&run));
    }
    // Guards against a vacuous pass: across many seeds the faults must
    // actually bite.
    if several {
        for (what, count) in coverage.counters() {
            assert!(count > 0, "no {what} across seeds {seeds:?}: {coverage:?}");
        }
    }
}

#[test]
fn a_seed_replays_to_the_byte() {
    for seed in [3, 17, 42] {
        let first = run(seed, Plan::standard());
        let second = run(seed, Plan::standard());
        assert_eq!(
            to_ndjson(&first.journal),
            to_ndjson(&second.journal),
            "seed {seed} wrote a different journal"
        );
        assert!(first == second, "seed {seed} is not deterministic");
    }
}
