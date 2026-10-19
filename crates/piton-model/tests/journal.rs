//! Journals: deterministic to the byte, replayable, and rejected when they
//! disagree with the harness.

#![cfg(not(piton_planted_bug))]

use piton_core::{Input, JournalEntry, Submission};
use piton_model::{DivergenceReason, WorldConfig, from_ndjson, to_ndjson, validate, walk};

const GOLDEN: &str = include_str!("journals/deep-walk.ndjson");

/// A fixed, well-mixed choice sequence (a 16-bit linear congruential
/// generator), so the golden journal never depends on a library's RNG.
fn choices() -> Vec<u16> {
    let mut state: u16 = 0x2545;
    (0..300)
        .map(|_| {
            state = state.wrapping_mul(25_173).wrapping_add(13_849);
            state
        })
        .collect()
}

fn generated() -> Vec<JournalEntry> {
    walk(&WorldConfig::deep(), &choices())
        .map_err(|(_, divergence)| divergence)
        .expect("the walk keeps the invariants")
}

const fn submissions(entry: &mut JournalEntry) -> &mut Vec<Submission> {
    match entry {
        JournalEntry::Handled { submissions, .. } | JournalEntry::Recovered { submissions, .. } => {
            submissions
        }
    }
}

#[test]
fn the_same_choices_give_a_byte_identical_journal() {
    assert_eq!(
        to_ndjson(&generated()),
        to_ndjson(&generated()),
        "two runs of one choice sequence differ"
    );
}

#[test]
fn a_generated_journal_replays() {
    let entries = from_ndjson(&to_ndjson(&generated())).expect("a journal parses");
    validate(&WorldConfig::deep(), &entries).expect("a generated journal replays");
}

#[test]
fn the_golden_journal_still_replays() {
    let entries = from_ndjson(GOLDEN).expect("the golden journal parses");
    assert!(
        entries.len() > 50,
        "the golden journal is suspiciously short"
    );
    validate(&WorldConfig::deep(), &entries).expect("the golden journal replays");
}

#[test]
fn a_tampered_journal_is_rejected_at_the_tampered_entry() {
    let mut entries = generated();
    let tampered = entries
        .iter_mut()
        .position(|entry| submissions(entry).len() > 1)
        .expect("some entry submits more than once");
    let entry = entries.get_mut(tampered).expect("the index is in range");
    submissions(entry).pop();
    let divergence = validate(&WorldConfig::deep(), &entries).expect_err("tampering is caught");
    assert_eq!(divergence.entry, tampered, "caught at the wrong entry");
    assert!(
        matches!(divergence.reason, DivergenceReason::Submissions { .. }),
        "caught for the wrong reason: {:?}",
        divergence.reason
    );
}

#[test]
fn an_acknowledgement_with_nothing_in_flight_is_rejected() {
    let mut entries = generated();
    let ack = entries
        .iter()
        .find(|entry| {
            matches!(
                entry,
                JournalEntry::Handled {
                    input: Input::Appended { .. },
                    ..
                }
            )
        })
        .expect("the walk acknowledges an append")
        .clone();
    entries.insert(1, ack);
    let divergence = validate(&WorldConfig::deep(), &entries).expect_err("the ack is impossible");
    assert_eq!(divergence.entry, 1, "caught at the wrong entry");
    assert_eq!(
        divergence.reason,
        DivergenceReason::Inadmissible,
        "caught for the wrong reason"
    );
}

/// Rewrites the golden journal. Only for deliberate, reviewed changes to the
/// core's behaviour or the journal format.
#[test]
#[ignore = "rewrites tests/journals/deep-walk.ndjson; run with --ignored to bless"]
fn bless_golden_journal() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/journals/deep-walk.ndjson"
    );
    std::fs::write(path, to_ndjson(&generated())).expect("the golden journal is writable");
}
