use std::fmt::Write as _;

use piton_core::Submission;
use piton_domain::Offset;
use serde::{Deserialize, Serialize};

use crate::{EnvAction, Violation, World, WorldConfig, check, enabled, step};

/// One line of a trace: what the environment did, what the session
/// submitted in response, and a summary of where the log stood after.
///
/// A trace is the shell's input journal plus the environment events a
/// journal cannot see, so traces from the model, the simulator and
/// production share one format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    pub action: EnvAction,
    pub submissions: Vec<Submission>,
    pub log_len: Offset,
    pub durable_len: Offset,
}

/// Where a trace and the harness disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// Index of the first step that diverged.
    pub step: usize,
    pub reason: DivergenceReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivergenceReason {
    /// The trace does something the environment model says cannot happen.
    NotEnabled,
    /// The session submitted something other than what the trace recorded.
    Submissions {
        recorded: Vec<Submission>,
        replayed: Vec<Submission>,
    },
    /// The log ended up a different length than the trace recorded.
    LogLength,
    Invariant(Violation),
}

/// Replays `steps` from a fresh world, checking each is allowed, reproduces
/// what was recorded, and keeps every invariant.
///
/// # Errors
///
/// Returns the first step where the trace and the harness disagree.
pub fn validate(config: &WorldConfig, steps: &[TraceStep]) -> Result<World, Divergence> {
    let mut world = World::new(config);
    check(config, &world).map_err(|violation| Divergence {
        step: 0,
        reason: DivergenceReason::Invariant(violation),
    })?;
    for (index, recorded) in steps.iter().enumerate() {
        let diverged = |reason| Divergence {
            step: index,
            reason,
        };
        if !enabled(config, &world).contains(&recorded.action) {
            return Err(diverged(DivergenceReason::NotEnabled));
        }
        let replayed = step(&mut world, recorded.action.clone());
        if replayed != recorded.submissions {
            return Err(diverged(DivergenceReason::Submissions {
                recorded: recorded.submissions.clone(),
                replayed,
            }));
        }
        if world.session().log_len() != recorded.log_len
            || world.session().durable_len() != recorded.durable_len
        {
            return Err(diverged(DivergenceReason::LogLength));
        }
        check(config, &world)
            .map_err(|violation| diverged(DivergenceReason::Invariant(violation)))?;
    }
    Ok(world)
}

/// Walks the world by picking, at each step, the enabled action indexed by
/// the next choice (wrapping). Stops at the first violation or once the
/// environment can only let time pass.
///
/// # Errors
///
/// Returns the trace so far alongside the first broken invariant.
pub fn walk(
    config: &WorldConfig,
    choices: &[u16],
) -> Result<Vec<TraceStep>, (Vec<TraceStep>, Divergence)> {
    let mut world = World::new(config);
    let mut steps = Vec::new();
    for (index, choice) in choices.iter().enumerate() {
        let actions = enabled(config, &world);
        if actions
            .iter()
            .all(|action| matches!(action, EnvAction::Advance { .. }))
        {
            break;
        }
        let Some(action) = usize::from(*choice)
            .checked_rem(actions.len())
            .and_then(|picked| actions.get(picked))
        else {
            break;
        };
        let submissions = step(&mut world, action.clone());
        steps.push(TraceStep {
            action: action.clone(),
            submissions,
            log_len: world.session().log_len(),
            durable_len: world.session().durable_len(),
        });
        if let Err(violation) = check(config, &world) {
            let divergence = Divergence {
                step: index,
                reason: DivergenceReason::Invariant(violation),
            };
            return Err((steps, divergence));
        }
    }
    Ok(steps)
}

/// Serializes a trace as newline-delimited JSON, one step per line.
///
/// # Panics
///
/// Panics if a step fails to serialize, which the types rule out.
#[must_use]
pub fn to_ndjson(steps: &[TraceStep]) -> String {
    let mut text = String::new();
    for step in steps {
        let line = serde_json::to_string(step).expect("a trace step serializes");
        writeln!(text, "{line}").expect("writing to a String cannot fail");
    }
    text
}

/// Parses newline-delimited JSON written by [`to_ndjson`].
///
/// # Errors
///
/// Returns the parse error of the first malformed line.
pub fn from_ndjson(text: &str) -> Result<Vec<TraceStep>, serde_json::Error> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(serde_json::from_str)
        .collect()
}
