use std::fmt::Write as _;

use piton_core::{Input, JournalEntry, Submission};
use piton_domain::{Offset, Timestamp};

use crate::{
    EnvAction, Violation, World, WorldConfig, admits, check, check_exploration, enabled, recovery,
    step,
};

/// Where a journal and the harness disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// Index of the first entry that diverged.
    pub entry: usize,
    pub reason: DivergenceReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivergenceReason {
    /// The journal does not begin with a freshly created session.
    NotFromCreation,
    /// The journal shows the shell doing something the contract forbids.
    Inadmissible,
    /// The session restarted from a log no sequence of storage outcomes
    /// explains.
    UnexplainedRecovery {
        log_len: Offset,
    },
    /// The session submitted something other than what the journal recorded.
    Submissions {
        recorded: Vec<Submission>,
        replayed: Vec<Submission>,
    },
    Invariant(Violation),
}

/// Replays a session's journal through the harness from its creation.
///
/// Every step must be one the contract admits, the session must submit
/// exactly what was recorded, and every invariant must hold.
///
/// Journals from any source replay alike: model walks, the simulator, or a
/// production shell. `config` supplies the session's tools and model policy.
///
/// # Errors
///
/// Returns the first entry where the journal and the harness disagree.
pub fn validate(config: &WorldConfig, entries: &[JournalEntry]) -> Result<World, Divergence> {
    let diverged = |entry, reason| Divergence { entry, reason };
    let Some(JournalEntry::Recovered {
        now,
        log_len,
        submissions,
    }) = entries.first()
    else {
        return Err(diverged(0, DivergenceReason::NotFromCreation));
    };
    let (mut world, start) = World::start(config, *now);
    let JournalEntry::Recovered {
        log_len: created_len,
        submissions: replayed,
        ..
    } = start
    else {
        return Err(diverged(0, DivergenceReason::NotFromCreation));
    };
    if *log_len != created_len {
        return Err(diverged(0, DivergenceReason::NotFromCreation));
    }
    compare(submissions, replayed).map_err(|reason| diverged(0, reason))?;
    check(&world).map_err(|violation| diverged(0, DivergenceReason::Invariant(violation)))?;

    for (index, entry) in entries.iter().enumerate().skip(1) {
        replay(&mut world, entry).map_err(|reason| diverged(index, reason))?;
        check(&world)
            .map_err(|violation| diverged(index, DivergenceReason::Invariant(violation)))?;
    }
    Ok(world)
}

fn replay(world: &mut World, entry: &JournalEntry) -> Result<(), DivergenceReason> {
    match entry {
        JournalEntry::Handled {
            now,
            input,
            submissions,
        } => {
            world.advance_to(*now);
            let action = match input {
                Input::Tick => EnvAction::Tick { at: *now },
                Input::Appended { .. }
                | Input::ModelResponded { .. }
                | Input::ModelFailed { .. }
                | Input::ToolReturned { .. }
                | Input::EffectCancelled { .. }
                | Input::Submit { .. }
                | Input::CancelTurn { .. } => EnvAction::Deliver(input.clone()),
            };
            if !admits(world, &action) {
                return Err(DivergenceReason::Inadmissible);
            }
            compare(submissions, replayed(step(world, action)))
        }
        JournalEntry::Recovered {
            now,
            log_len,
            submissions,
        } => {
            world.advance_to(*now);
            let actions = recovery(world, *log_len)
                .ok_or(DivergenceReason::UnexplainedRecovery { log_len: *log_len })?;
            let mut last = None;
            for action in actions {
                last = step(world, action);
            }
            compare(submissions, replayed(last))
        }
    }
}

fn replayed(entry: Option<JournalEntry>) -> Vec<Submission> {
    match entry {
        Some(
            JournalEntry::Handled { submissions, .. } | JournalEntry::Recovered { submissions, .. },
        ) => submissions,
        None => Vec::new(),
    }
}

fn compare(recorded: &[Submission], replayed: Vec<Submission>) -> Result<(), DivergenceReason> {
    if recorded == replayed.as_slice() {
        Ok(())
    } else {
        Err(DivergenceReason::Submissions {
            recorded: recorded.to_vec(),
            replayed,
        })
    }
}

/// Walks the world from creation and returns the journal a shell would
/// have written.
///
/// At each step it picks the enabled action indexed by the next choice
/// (wrapping). It stops at the first broken invariant or once nothing more
/// can happen.
///
/// # Errors
///
/// Returns the journal so far alongside the first broken invariant.
pub fn walk(
    config: &WorldConfig,
    choices: &[u16],
) -> Result<Vec<JournalEntry>, (Vec<JournalEntry>, Divergence)> {
    let (mut world, start) = World::start(config, Timestamp::EPOCH);
    let mut entries = Vec::from([start]);
    for choice in choices {
        let actions = enabled(config, &world);
        let Some(action) = usize::from(*choice)
            .checked_rem(actions.len())
            .and_then(|picked| actions.get(picked))
        else {
            break;
        };
        entries.extend(step(&mut world, action.clone()));
        if let Err(violation) = check(&world).and_then(|()| check_exploration(config, &world)) {
            let divergence = Divergence {
                entry: entries.len().saturating_sub(1),
                reason: DivergenceReason::Invariant(violation),
            };
            return Err((entries, divergence));
        }
    }
    Ok(entries)
}

/// Serializes a journal as newline-delimited JSON, one entry per line.
///
/// # Panics
///
/// Panics if an entry fails to serialize, which the types rule out.
#[must_use]
pub fn to_ndjson(entries: &[JournalEntry]) -> String {
    let mut text = String::new();
    for entry in entries {
        let line = serde_json::to_string(entry).expect("a journal entry serializes");
        writeln!(text, "{line}").expect("writing to a String cannot fail");
    }
    text
}

/// Parses newline-delimited JSON written by [`to_ndjson`].
///
/// # Errors
///
/// Returns the parse error of the first malformed line.
pub fn from_ndjson(text: &str) -> Result<Vec<JournalEntry>, serde_json::Error> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(serde_json::from_str)
        .collect()
}
