//! Traces: deterministic to the byte, replayable, and rejected when they
//! disagree with the harness.

#![cfg(not(piton_planted_bug))]

use piton_model::{
    DivergenceReason, TraceStep, WorldConfig, from_ndjson, to_ndjson, validate, walk,
};

const GOLDEN: &str = include_str!("traces/deep-walk.ndjson");

/// A fixed, well-mixed choice sequence (a 16-bit linear congruential
/// generator), so the golden trace never depends on a library's RNG.
fn choices() -> Vec<u16> {
    let mut state: u16 = 0x2545;
    (0..300)
        .map(|_| {
            state = state.wrapping_mul(25_173).wrapping_add(13_849);
            state
        })
        .collect()
}

fn generated() -> Vec<TraceStep> {
    walk(&WorldConfig::deep(), &choices())
        .map_err(|(_, divergence)| divergence)
        .expect("the walk keeps the invariants")
}

#[test]
fn the_same_choices_give_a_byte_identical_trace() {
    assert_eq!(
        to_ndjson(&generated()),
        to_ndjson(&generated()),
        "two runs of one choice sequence differ"
    );
}

#[test]
fn a_generated_trace_replays() {
    let steps = from_ndjson(&to_ndjson(&generated())).expect("a trace parses");
    validate(&WorldConfig::deep(), &steps).expect("a generated trace replays");
}

#[test]
fn the_golden_trace_still_replays() {
    let steps = from_ndjson(GOLDEN).expect("the golden trace parses");
    assert!(steps.len() > 50, "the golden trace is suspiciously short");
    validate(&WorldConfig::deep(), &steps).expect("the golden trace replays");
}

#[test]
fn a_tampered_trace_is_rejected_at_the_tampered_step() {
    let mut steps = generated();
    let tampered = steps
        .iter()
        .position(|step| step.submissions.len() > 1)
        .expect("some step submits more than once");
    let tampered_step = steps.get_mut(tampered).expect("the index is in range");
    tampered_step.submissions.pop();
    let divergence = validate(&WorldConfig::deep(), &steps).expect_err("tampering is caught");
    assert_eq!(divergence.step, tampered, "caught at the wrong step");
    assert!(
        matches!(divergence.reason, DivergenceReason::Submissions { .. }),
        "caught for the wrong reason: {:?}",
        divergence.reason
    );
}

#[test]
fn an_impossible_action_is_rejected() {
    let mut steps = generated();
    steps.insert(0, steps.last().expect("the trace is not empty").clone());
    let divergence = validate(&WorldConfig::deep(), &steps).expect_err("the action is not enabled");
    assert_eq!(divergence.step, 0, "caught at the wrong step");
    assert_eq!(
        divergence.reason,
        DivergenceReason::NotEnabled,
        "caught for the wrong reason"
    );
}

/// Rewrites the golden trace. Only for deliberate, reviewed changes to the
/// core's behaviour or the trace format.
#[test]
#[ignore = "rewrites tests/traces/deep-walk.ndjson; run with --ignored to bless"]
fn bless_golden_trace() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/traces/deep-walk.ndjson");
    std::fs::write(path, to_ndjson(&generated())).expect("the golden trace is writable");
}
