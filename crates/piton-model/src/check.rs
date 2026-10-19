use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use piton_core::SessionState;
use piton_domain::{EffectId, Offset, Record, RecordBody};

use crate::{EnvAction, World, WorldConfig, enabled};

/// A broken invariant.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Violation {
    /// Folding the records the session produced does not give its state.
    ReplayDiverged,
    Unrecoverable(String),
    DispatchBeforeDurable(EffectId),
    DispatchMismatch(EffectId),
    DoubleDispatch(EffectId),
    UnsafeRedispatch(EffectId),
    CancelUndispatched(EffectId),
    ConcurrentAppend,
    AppendNotAtTail {
        expected: Offset,
        tail: Offset,
    },
    ResolvedWithoutIntent(EffectId),
    ResolvedTwice(EffectId),
    MalformedModelRequest {
        effect: EffectId,
        reason: String,
    },
    /// Nothing more can happen, yet the session has unfinished work.
    StuckAtQuiescence,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Violation {}

/// Checks every invariant against the current state of `world`.
///
/// # Errors
///
/// Returns the first invariant `world` breaks.
pub fn check(config: &WorldConfig, world: &World) -> Result<(), Violation> {
    let env = world.env();
    let session = world.session();
    if let Some(violation) = env.violation() {
        return Err(violation.clone());
    }

    let produced: Vec<&Record> = env
        .durable()
        .iter()
        .chain(env.in_flight())
        .chain(session.unsubmitted())
        .collect();

    // State is a fold of the log: replaying everything the session produced
    // reproduces it exactly, and the durable prefix always recovers.
    match SessionState::fold(produced.iter().copied()) {
        Ok(replayed) if &replayed == session.state() => {}
        Ok(_) | Err(_) => return Err(Violation::ReplayDiverged),
    }
    SessionState::fold(env.durable())
        .map_err(|error| Violation::Unrecoverable(error.to_string()))?;

    // Each intent is resolved at most once, by a resolution of its own kind,
    // and every model call carries a request providers accept.
    let mut intents = BTreeMap::new();
    let mut resolved = BTreeSet::new();
    for record in &produced {
        let id = EffectId::new(record.offset);
        match &record.body {
            RecordBody::ModelIntent { request, .. } => {
                request
                    .validate()
                    .map_err(|reason| Violation::MalformedModelRequest {
                        effect: id,
                        reason: reason.to_string(),
                    })?;
                intents.insert(id, true);
            }
            RecordBody::ToolIntent { .. } => {
                intents.insert(id, false);
            }
            RecordBody::ModelResolved { effect, .. } | RecordBody::ToolResolved { effect, .. } => {
                let is_model = matches!(record.body, RecordBody::ModelResolved { .. });
                if intents.get(effect) != Some(&is_model) {
                    return Err(Violation::ResolvedWithoutIntent(*effect));
                }
                if !resolved.insert(*effect) {
                    return Err(Violation::ResolvedTwice(*effect));
                }
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::InputAdmitted { .. }
            | RecordBody::TurnStarted { .. }
            | RecordBody::TurnCancelled { .. }
            | RecordBody::TurnEnded { .. } => {}
        }
    }

    // Liveness as safety: once the environment can do nothing but let time
    // pass, every turn has ended and every record is durable.
    let quiescent = enabled(config, world)
        .iter()
        .all(|action| matches!(action, EnvAction::Advance { .. }));
    if quiescent
        && (session.active_turn().is_some()
            || !env.in_flight().is_empty()
            || !session.unsubmitted().is_empty()
            || resolved.len() != intents.len())
    {
        return Err(Violation::StuckAtQuiescence);
    }
    Ok(())
}
