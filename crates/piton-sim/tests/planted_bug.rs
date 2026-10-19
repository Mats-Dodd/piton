//! Proof the simulator can fail. Built only with
//! `--cfg piton_planted_shell_bug`, which makes the runtime leave abandoned
//! tools running (`just planted-bug`). Only faults that make the session
//! abandon tools expose it.

#![cfg(all(tokio_unstable, piton_planted_shell_bug))]

use piton_sim::{Failure, Plan, check, run};

#[test]
fn the_simulator_finds_the_planted_bug() {
    let found = (0..64).find_map(|seed| {
        let run = run(seed, Plan::standard());
        check(&run).err().map(|failure| (seed, failure))
    });
    assert!(
        matches!(
            found,
            Some((_, Failure::Untruthful(_) | Failure::Diverged(_)))
        ),
        "the simulator missed a runtime that leaves abandoned tools running: {found:?}"
    );
}
