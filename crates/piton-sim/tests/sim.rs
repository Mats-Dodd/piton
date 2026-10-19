//! Seeded simulations of the runtime under faults (`just sim`).

#![cfg(all(tokio_unstable, not(piton_planted_shell_bug)))]

use piton_model::to_ndjson;
use piton_sim::{Coverage, Plan, Run, check, run};

/// Seeds to run: 256 by default, or `PITON_SIM_SEEDS` for longer runs. Some
/// races only show up in about one seed in a thousand.
fn seeds() -> u64 {
    std::env::var("PITON_SIM_SEEDS")
        .ok()
        .and_then(|seeds| seeds.parse().ok())
        .unwrap_or(256)
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
    let mut coverage = Coverage::default();
    let seeds = seeds();
    for seed in 0..seeds {
        let run = run(seed, Plan::standard()).expect("the simulation runs");
        let checked = check(&run);
        assert!(
            checked.is_ok(),
            "seed {seed} failed: {:?}\njournal: {}",
            checked.err(),
            keep(&run)
        );
        coverage = coverage.add(Coverage::of(&run));
    }
    // Guards against a vacuous pass: the faults must actually bite.
    let Coverage {
        recoveries,
        crashes,
        late_landed,
        retries,
        timeouts,
        stopped,
        abandoned,
        not_run,
        restarted,
    } = coverage;
    for (what, count) in [
        ("recoveries", recoveries),
        ("crashes", crashes),
        ("late appends landing", late_landed),
        ("retries", retries),
        ("timeouts", timeouts),
        ("confirmed stops", stopped),
        ("abandoned tools", abandoned),
        ("tools never run", not_run),
        ("tools interrupted by a restart", restarted),
    ] {
        assert!(count > 0, "no {what} across {seeds} seeds: {coverage:?}");
    }
}

#[test]
fn a_seed_replays_to_the_byte() {
    for seed in [3, 17, 42] {
        let first = run(seed, Plan::standard()).expect("the simulation runs");
        let second = run(seed, Plan::standard()).expect("the simulation runs");
        assert_eq!(
            to_ndjson(&first.journal),
            to_ndjson(&second.journal),
            "seed {seed} is not deterministic"
        );
        assert_eq!(first.log, second.log, "seed {seed} left a different log");
    }
}
