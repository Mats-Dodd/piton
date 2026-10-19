//! Proof the simulator can fail. Built only with
//! `--cfg piton_planted_shell_bug`, which makes the runtime acknowledge
//! appends before storage has them (`just planted-bug`).

#![cfg(all(tokio_unstable, piton_planted_shell_bug))]

use piton_sim::{Failure, Plan, check, run};

#[test]
fn the_simulator_finds_the_planted_bug() {
    let found = (0..64).find_map(|seed| {
        let run = run(seed, Plan::standard()).expect("the simulation runs");
        check(&run).err().map(|failure| (seed, failure))
    });
    assert!(
        matches!(
            found,
            Some((_, Failure::Diverged(_) | Failure::Untruthful(_)))
        ),
        "the simulator missed a runtime that acknowledges unwritten appends: {found:?}"
    );
}
