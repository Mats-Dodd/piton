//! Proof the checkers can fail. Built only with `--cfg piton_planted_bug`,
//! which makes the core dispatch effects before their intent is durable
//! (`just planted-bug`).

#![cfg(piton_planted_bug)]

use piton_domain::Timestamp;
use piton_model::{Checked, DivergenceReason, Violation, World, WorldConfig, check, step, walk};
use proptest::{
    collection::vec,
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestError, TestRng, TestRunner},
};
use stateright::{Checker as _, HasDiscoveries, Model as _};

#[test]
fn stateright_finds_the_planted_bug() {
    let checker = Checked(WorldConfig::exhaustive())
        .checker()
        .finish_when(HasDiscoveries::AnyFailures)
        .spawn_bfs()
        .join();
    let counterexample = checker
        .discovery("invariants hold")
        .expect("Stateright finds a state that breaks the invariants");
    // Stateright 0.31 lets later discoveries overwrite earlier ones, so the
    // path is a counterexample but not necessarily a shortest one. Replay it
    // to find the first step that breaks.
    let mut world = World::new(&WorldConfig::exhaustive(), Timestamp::EPOCH);
    let first_break = counterexample
        .into_actions()
        .into_iter()
        .enumerate()
        .find_map(|(index, action)| {
            step(&mut world, action);
            check(&world).err().map(|violation| (index, violation))
        });
    assert!(
        matches!(first_break, Some((0, Violation::DispatchBeforeDurable(_)))),
        "the first submission dispatches early, got {first_break:?}"
    );
}

#[test]
fn proptest_finds_and_shrinks_the_planted_bug() {
    let mut runner = TestRunner::new_with_rng(
        Config {
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let outcome = runner.run(&vec(any::<u16>(), 0..400), |choices| {
        let walked = walk(&WorldConfig::deep(), &choices);
        prop_assert!(walked.is_ok(), "an invariant broke");
        Ok(())
    });
    let Err(TestError::Fail(_, minimal)) = outcome else {
        panic!("proptest missed the planted bug: {outcome:?}");
    };
    let Err((entries, divergence)) = walk(&WorldConfig::deep(), &minimal) else {
        panic!("the shrunk input no longer fails");
    };
    // The creation entry, then the submit that dispatches too early.
    assert!(entries.len() <= 2, "shrunk to {} entries", entries.len());
    assert!(
        matches!(
            divergence.reason,
            DivergenceReason::Invariant(Violation::DispatchBeforeDurable(_))
        ),
        "found for the wrong reason: {:?}",
        divergence.reason
    );
}
