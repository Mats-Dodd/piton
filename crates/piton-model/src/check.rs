use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use piton_core::SessionState;
use piton_domain::{
    EffectId, Millis, ModelOutcome, ModelPolicy, Offset, Record, RecordBody, Safety, StopReason,
    Timestamp, ToolName, ToolOutcome, ToolSpec, TurnId, UnknownCause,
};

use crate::{World, WorldConfig, admits, enabled};

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
    AbandonUndispatched(EffectId),
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
    /// A tool reported as not run had been dispatched.
    UntruthfulNotRun(EffectId),
    /// An unsafe tool reported as stopped never confirmed it.
    UnconfirmedStop(EffectId),
    /// A timeout or grace period was cut short.
    PrematureTimeout(EffectId),
    /// A model call was retried before its backoff passed.
    PrematureRetry(EffectId),
    /// A turn made more model calls in a row than its retry policy allows.
    TooManyAttempts(TurnId),
    /// The session's deadline is not in its future.
    StaleDeadline,
    /// The generator explores something the contract forbids.
    GeneratorExceedsContract,
    /// Nothing more can happen, yet the session has unfinished work.
    StuckAtQuiescence,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Violation {}

/// What the checks need to know about an effect's intent.
struct Intent {
    at: Timestamp,
    turn: TurnId,
    /// `None` for model calls.
    tool: Option<(Safety, Option<ToolSpec>)>,
}

/// Checks the safety invariants: they hold in every state of every run,
/// whether explored, walked or replayed from a shell's journal.
///
/// # Errors
///
/// Returns the first invariant `world` breaks.
pub fn check(world: &World) -> Result<(), Violation> {
    let env = world.env();
    let session = world.session();
    if let Some(violation) = env.violation() {
        return Err(violation.clone());
    }

    let produced: Vec<&Record> = env.known().chain(session.unsubmitted()).collect();

    // State is a fold of the log: replaying everything the session produced
    // reproduces it exactly, and what storage holds always recovers.
    match SessionState::fold(produced.iter().copied()) {
        Ok(replayed) if &replayed == session.state() => {}
        Ok(_) | Err(_) => return Err(Violation::ReplayDiverged),
    }
    SessionState::fold(env.durable())
        .map_err(|error| Violation::Unrecoverable(error.to_string()))?;

    let (tools, model) = match produced.first().map(|record| &record.body) {
        Some(RecordBody::SessionCreated { tools, model, .. }) => (
            tools
                .iter()
                .map(|spec| (spec.name.clone(), spec.clone()))
                .collect::<BTreeMap<ToolName, ToolSpec>>(),
            *model,
        ),
        _ => return Err(Violation::ReplayDiverged),
    };

    let mut intents: BTreeMap<EffectId, Intent> = BTreeMap::new();
    let mut resolved = BTreeSet::new();
    let mut cancelled: BTreeMap<TurnId, Timestamp> = BTreeMap::new();
    // Per turn: consecutive failures, and when the last one allows a retry.
    let mut failures: BTreeMap<TurnId, (u32, Timestamp)> = BTreeMap::new();
    for record in &produced {
        let id = EffectId::new(record.offset);
        match &record.body {
            RecordBody::ModelIntent { turn, request } => {
                request
                    .validate()
                    .map_err(|reason| Violation::MalformedModelRequest {
                        effect: id,
                        reason: reason.to_string(),
                    })?;
                if let Some((_, retry_at)) = failures.get(turn)
                    && record.at < *retry_at
                {
                    return Err(Violation::PrematureRetry(id));
                }
                intents.insert(
                    id,
                    Intent {
                        at: record.at,
                        turn: *turn,
                        tool: None,
                    },
                );
            }
            RecordBody::ToolIntent { turn, call, safety } => {
                intents.insert(
                    id,
                    Intent {
                        at: record.at,
                        turn: *turn,
                        tool: Some((*safety, tools.get(&call.name).cloned())),
                    },
                );
            }
            RecordBody::ModelResolved { effect, outcome } => {
                let intent = resolve(&intents, &mut resolved, *effect, true)?;
                check_model_outcome(&model, intent, record, &mut failures, outcome)?;
            }
            RecordBody::ToolResolved { effect, outcome } => {
                let intent = resolve(&intents, &mut resolved, *effect, false)?;
                check_tool_outcome(world, &cancelled, *effect, intent, record.at, outcome)?;
            }
            RecordBody::TurnCancelled { turn } => {
                cancelled.entry(*turn).or_insert(record.at);
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::SessionResumed
            | RecordBody::InputAdmitted { .. }
            | RecordBody::TurnStarted { .. }
            | RecordBody::TurnEnded { .. } => {}
        }
    }

    // A deadline already passed would leave the shell ticking for nothing.
    if session
        .deadline()
        .is_some_and(|deadline| deadline <= env.now())
    {
        return Err(Violation::StaleDeadline);
    }
    Ok(())
}

/// Checks that only make sense while exploring with a generator: it stays
/// within the contract, and once it can do nothing more, the session has
/// finished everything it started.
///
/// # Errors
///
/// Returns the first check `world` fails.
pub fn check_exploration(config: &WorldConfig, world: &World) -> Result<(), Violation> {
    let actions = enabled(config, world);
    if !actions.iter().all(|action| admits(world, action)) {
        return Err(Violation::GeneratorExceedsContract);
    }
    let session = world.session();
    let mut open = BTreeSet::new();
    for record in world.env().known().chain(session.unsubmitted()) {
        if let RecordBody::ModelResolved { effect, .. } | RecordBody::ToolResolved { effect, .. } =
            &record.body
        {
            open.remove(effect);
        } else if let RecordBody::ModelIntent { .. } | RecordBody::ToolIntent { .. } = &record.body
        {
            open.insert(EffectId::new(record.offset));
        }
    }
    let unresolved = !open.is_empty();
    if actions.is_empty()
        && (session.active_turn().is_some()
            || world.env().in_flight()
            || !session.unsubmitted().is_empty()
            || session.deadline().is_some()
            || unresolved)
    {
        return Err(Violation::StuckAtQuiescence);
    }
    Ok(())
}

/// Marks `effect` resolved, returning its intent.
fn resolve<'intent>(
    intents: &'intent BTreeMap<EffectId, Intent>,
    resolved: &mut BTreeSet<EffectId>,
    effect: EffectId,
    is_model: bool,
) -> Result<&'intent Intent, Violation> {
    let intent = intents
        .get(&effect)
        .filter(|intent| intent.tool.is_none() == is_model)
        .ok_or(Violation::ResolvedWithoutIntent(effect))?;
    if !resolved.insert(effect) {
        return Err(Violation::ResolvedTwice(effect));
    }
    Ok(intent)
}

/// Retries stay within the policy and wait out their backoff.
fn check_model_outcome(
    model: &ModelPolicy,
    intent: &Intent,
    record: &Record,
    failures: &mut BTreeMap<TurnId, (u32, Timestamp)>,
    outcome: &ModelOutcome,
) -> Result<(), Violation> {
    match outcome {
        ModelOutcome::Failed { retry_after, .. } => {
            let count = failures
                .get(&intent.turn)
                .map_or(0, |(count, _)| *count)
                .saturating_add(1);
            if count > model.retry.max_attempts {
                return Err(Violation::TooManyAttempts(intent.turn));
            }
            let wait = model
                .retry
                .backoff(count)
                .max(retry_after.unwrap_or(Millis::ZERO));
            failures.insert(intent.turn, (count, record.at.after(wait)));
        }
        ModelOutcome::Responded { .. } => {
            failures.remove(&intent.turn);
        }
        ModelOutcome::Cancelled => {}
    }
    Ok(())
}

/// Tool outcomes tell the truth, and time-driven ones come no earlier than
/// their deadlines.
fn check_tool_outcome(
    world: &World,
    cancelled: &BTreeMap<TurnId, Timestamp>,
    effect: EffectId,
    intent: &Intent,
    at: Timestamp,
    outcome: &ToolOutcome,
) -> Result<(), Violation> {
    let Some((safety, spec)) = &intent.tool else {
        return Ok(());
    };
    let timeout = spec.as_ref().map(|spec| intent.at.after(spec.timeout));
    let premature = |due: Option<Timestamp>| due.is_some_and(|due| at < due);
    match outcome {
        ToolOutcome::NotRun { reason } => {
            if world.env().started(effect) {
                return Err(Violation::UntruthfulNotRun(effect));
            }
            if *reason == StopReason::TimedOut && premature(timeout) {
                return Err(Violation::PrematureTimeout(effect));
            }
        }
        ToolOutcome::Stopped { reason } => {
            if *safety == Safety::Unsafe && !world.env().stopped(effect) {
                return Err(Violation::UnconfirmedStop(effect));
            }
            if *reason == StopReason::TimedOut && premature(timeout) {
                return Err(Violation::PrematureTimeout(effect));
            }
        }
        ToolOutcome::Unknown {
            cause: UnknownCause::Abandoned,
        } => {
            let grace = spec.as_ref().map(|spec| spec.cancel_grace);
            let after_timeout = timeout.zip(grace).map(|(due, grace)| due.after(grace));
            let after_cancel = cancelled
                .get(&intent.turn)
                .zip(grace)
                .map(|(due, grace)| due.after(grace));
            let earliest = match (after_timeout, after_cancel) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            };
            if premature(earliest) {
                return Err(Violation::PrematureTimeout(effect));
            }
        }
        ToolOutcome::Returned { .. }
        | ToolOutcome::Unknown {
            cause: UnknownCause::Restart,
        }
        | ToolOutcome::UnknownTool => {}
    }
    Ok(())
}
