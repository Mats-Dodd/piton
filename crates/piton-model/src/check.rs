use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use piton_core::{Input, SessionState};
use piton_domain::{
    EffectId, Millis, ModelOutcome, ModelPolicy, Offset, Record, RecordBody, RequestId, Safety,
    StopReason, Timestamp, ToolName, ToolOutcome, ToolSpec, TurnId, UnknownCause,
};

use crate::{World, WorldConfig, admits, enabled, makes_progress};

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
    /// An effect of a cancelled turn was dispatched after the cancellation.
    DispatchAfterCancel(EffectId),
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
    /// A request was admitted more than once.
    AdmittedTwice(RequestId),
    /// A turn started for a request that was not admitted, or that already
    /// had a turn.
    TurnWithoutInput(TurnId),
    /// A recorded outcome is not what the environment delivered, or claims
    /// a cause that did not happen.
    UnfaithfulOutcome(EffectId),
    /// A tool reported as not run had been dispatched.
    UntruthfulNotRun(EffectId),
    /// A tool reported as stopped never confirmed it.
    UnconfirmedStop(EffectId),
    /// A stop or skip is put down to a cancellation that did not happen.
    WrongStopReason(EffectId),
    /// An unsafe tool was abandoned without first being asked to stop.
    MissingCancel(EffectId),
    /// A timeout or grace period was cut short.
    PrematureTimeout(EffectId),
    /// A model call was retried before its backoff passed.
    PrematureRetry(EffectId),
    /// A model call was retried after a failure that ended its attempts.
    RetryAfterFatalFailure(EffectId),
    /// A turn made more model calls in a row than its retry policy allows.
    TooManyAttempts(TurnId),
    /// An effect's time ran out but the session has not resolved it.
    Overdue(EffectId),
    /// A result was used although it arrived after the effect's time ran
    /// out; timers due by an input's time fire before the input.
    LateResult(EffectId),
    /// An effect the session resolved without it is still running: the
    /// session never told the shell to drop it.
    ResolvedButRunning(EffectId),
    /// The session asks to be woken later than its next timer.
    LateDeadline,
    /// A turn's backoff ended but the session has not retried.
    RetryOverdue(TurnId),
    /// A turn ran out of attempts but has not ended.
    TurnNotEnded(TurnId),
    /// No turn is running, yet an admitted request waits.
    IdleWithQueue,
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
    offset: Offset,
    at: Timestamp,
    turn: TurnId,
    /// `None` for model calls; the spec is `None` for unknown tools.
    tool: Option<(Safety, Option<ToolSpec>)>,
}

/// What the checks track about a turn.
#[derive(Default)]
struct TurnFacts {
    cancelled: Option<(Offset, Timestamp)>,
    ended: bool,
    /// Consecutive failed model calls, and when the last allows a retry.
    failures: u32,
    retry_at: Option<Timestamp>,
    /// The last failure was not retryable, or used the last attempt.
    finished_failing: bool,
}

/// Everything learned from one scan of a log.
#[derive(Default)]
struct Scan {
    intents: BTreeMap<EffectId, Intent>,
    resolved: BTreeSet<EffectId>,
    turns: BTreeMap<TurnId, TurnFacts>,
    admitted: BTreeSet<RequestId>,
    started: BTreeSet<RequestId>,
    resumed: Vec<Offset>,
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

    // The log the session knows and the log storage holds both tell the
    // truth; they differ only while an append is outstanding or after one
    // from an earlier life landed late.
    let known = scan(world, &produced)?;
    let stored: Vec<&Record> = env.durable().iter().collect();
    scan(world, &stored)?;

    check_dispatches(world, &known)?;
    check_nothing_overdue(world, &known, &produced)?;
    if let Some(effect) = known.resolved.iter().find(|effect| env.running(**effect)) {
        return Err(Violation::ResolvedButRunning(*effect));
    }

    if session
        .deadline()
        .is_some_and(|deadline| deadline <= session.now())
    {
        return Err(Violation::StaleDeadline);
    }
    Ok(())
}

/// Checks that only make sense while exploring with a generator: it stays
/// within the contract, and once nothing but optional faults can happen,
/// the session has finished everything it started.
///
/// # Errors
///
/// Returns the first check `world` fails.
pub fn check_exploration(config: &WorldConfig, world: &World) -> Result<(), Violation> {
    let actions = enabled(config, world);
    if !actions.iter().all(|action| admits(world, action)) {
        return Err(Violation::GeneratorExceedsContract);
    }
    if actions.iter().any(|action| makes_progress(world, action)) {
        return Ok(());
    }
    let session = world.session();
    let produced: Vec<&Record> = world.env().known().chain(session.unsubmitted()).collect();
    let known = scan(world, &produced)?;
    let unresolved = known
        .intents
        .keys()
        .any(|effect| !known.resolved.contains(effect));
    if session.active_turn().is_some()
        || world.env().in_flight()
        || !session.unsubmitted().is_empty()
        || session.deadline().is_some()
        || unresolved
        || known.admitted != known.started
    {
        return Err(Violation::StuckAtQuiescence);
    }
    Ok(())
}

/// Walks a log, checking every rule that holds record by record.
fn scan(world: &World, records: &[&Record]) -> Result<Scan, Violation> {
    let Some(RecordBody::SessionCreated { tools, model, .. }) =
        records.first().map(|record| &record.body)
    else {
        return Err(Violation::ReplayDiverged);
    };
    let tools: BTreeMap<ToolName, ToolSpec> = tools
        .iter()
        .map(|spec| (spec.name.clone(), spec.clone()))
        .collect();
    let mut scan = Scan::default();
    for record in records {
        let id = EffectId::new(record.offset);
        match &record.body {
            RecordBody::InputAdmitted { request_id, .. } => {
                if !scan.admitted.insert(request_id.clone()) {
                    return Err(Violation::AdmittedTwice(request_id.clone()));
                }
            }
            RecordBody::TurnStarted { request_id } => {
                let turn = TurnId::new(record.offset);
                if !scan.admitted.contains(request_id) || !scan.started.insert(request_id.clone()) {
                    return Err(Violation::TurnWithoutInput(turn));
                }
                scan.turns.insert(turn, TurnFacts::default());
            }
            RecordBody::ModelIntent { turn, request } => {
                request
                    .validate()
                    .map_err(|reason| Violation::MalformedModelRequest {
                        effect: id,
                        reason: reason.to_string(),
                    })?;
                let facts = scan.turns.entry(*turn).or_default();
                if facts.finished_failing {
                    return Err(Violation::RetryAfterFatalFailure(id));
                }
                if facts.retry_at.is_some_and(|retry_at| record.at < retry_at) {
                    return Err(Violation::PrematureRetry(id));
                }
                facts.retry_at = None;
                scan.intents.insert(
                    id,
                    Intent {
                        offset: record.offset,
                        at: record.at,
                        turn: *turn,
                        tool: None,
                    },
                );
            }
            RecordBody::ToolIntent { turn, call, safety } => {
                scan.intents.insert(
                    id,
                    Intent {
                        offset: record.offset,
                        at: record.at,
                        turn: *turn,
                        tool: Some((*safety, tools.get(&call.name).cloned())),
                    },
                );
            }
            RecordBody::ModelResolved { effect, outcome } => {
                let intent = resolve(&scan.intents, &mut scan.resolved, *effect, true)?;
                let facts = scan.turns.entry(intent.turn).or_default();
                check_model_outcome(world, model, intent, facts, record, *effect, outcome)?;
            }
            RecordBody::ToolResolved { effect, outcome } => {
                let intent = resolve(&scan.intents, &mut scan.resolved, *effect, false)?;
                let context = ToolContext {
                    effect: *effect,
                    at: record.at,
                    cancelled: scan
                        .turns
                        .get(&intent.turn)
                        .and_then(|facts| facts.cancelled),
                    restarted: scan.resumed.iter().any(|resumed| intent.offset < *resumed),
                };
                check_tool_outcome(world, intent, &context, outcome)?;
            }
            RecordBody::TurnCancelled { turn } => {
                let facts = scan.turns.entry(*turn).or_default();
                facts.cancelled.get_or_insert((record.offset, record.at));
            }
            RecordBody::TurnEnded { turn, .. } => {
                scan.turns.entry(*turn).or_default().ended = true;
            }
            RecordBody::SessionResumed => scan.resumed.push(record.offset),
            RecordBody::SessionCreated { .. } => {}
        }
    }
    Ok(scan)
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

/// A model outcome is what the provider delivered, or a timeout or
/// cancellation that really happened; retries stay within the policy.
fn check_model_outcome(
    world: &World,
    model: &ModelPolicy,
    intent: &Intent,
    facts: &mut TurnFacts,
    record: &Record,
    effect: EffectId,
    outcome: &ModelOutcome,
) -> Result<(), Violation> {
    let delivered = world.env().delivered(effect);
    match outcome {
        ModelOutcome::Responded { response } => {
            let faithful = delivered.iter().any(|input| {
                matches!(input, Input::ModelResponded { response: given, .. } if given == response)
            });
            if !faithful {
                return Err(Violation::UnfaithfulOutcome(effect));
            }
            if facts.cancelled.is_some() || record.at >= intent.at.after(model.timeout) {
                return Err(Violation::LateResult(effect));
            }
            facts.failures = 0;
        }
        ModelOutcome::Failed {
            reason,
            retryable,
            retry_after,
        } => {
            let reported = delivered.iter().any(|input| {
                matches!(input, Input::ModelFailed { reason: given, retryable: given_retryable, retry_after: given_after, .. }
                    if given == reason && given_retryable == retryable && given_after == retry_after)
            });
            // A reply the session judged unusable fails for good.
            let rejected_reply = !retryable
                && retry_after.is_none()
                && delivered
                    .iter()
                    .any(|input| matches!(input, Input::ModelResponded { .. }));
            if reported || rejected_reply {
                if facts.cancelled.is_some() || record.at >= intent.at.after(model.timeout) {
                    return Err(Violation::LateResult(effect));
                }
            } else {
                // Otherwise it can only be the session's own timeout.
                if !retryable || retry_after.is_some() {
                    return Err(Violation::UnfaithfulOutcome(effect));
                }
                if record.at < intent.at.after(model.timeout) {
                    return Err(Violation::PrematureTimeout(effect));
                }
            }
            facts.failures = facts.failures.saturating_add(1);
            if facts.failures > model.retry.max_attempts {
                return Err(Violation::TooManyAttempts(intent.turn));
            }
            let wait = model
                .retry
                .backoff(facts.failures)
                .max(retry_after.unwrap_or(Millis::ZERO));
            facts.retry_at = Some(record.at.after(wait));
            facts.finished_failing = !retryable || facts.failures >= model.retry.max_attempts;
        }
        ModelOutcome::Cancelled => {
            if facts
                .cancelled
                .is_none_or(|(_, cancelled_at)| cancelled_at > record.at)
            {
                return Err(Violation::UnfaithfulOutcome(effect));
            }
        }
    }
    Ok(())
}

/// Where a tool outcome sits in its log.
struct ToolContext {
    effect: EffectId,
    at: Timestamp,
    /// The turn's cancellation, if it came before.
    cancelled: Option<(Offset, Timestamp)>,
    /// Whether the session restarted after the intent.
    restarted: bool,
}

/// Tool outcomes are what the tool delivered or what really happened to it,
/// and time-driven ones come no earlier than their deadlines.
fn check_tool_outcome(
    world: &World,
    intent: &Intent,
    context: &ToolContext,
    outcome: &ToolOutcome,
) -> Result<(), Violation> {
    let Some((safety, spec)) = &intent.tool else {
        return Ok(());
    };
    let effect = context.effect;
    let env = world.env();
    let Some(spec) = spec else {
        return if *outcome == ToolOutcome::UnknownTool {
            Ok(())
        } else {
            Err(Violation::UnfaithfulOutcome(effect))
        };
    };
    let timeout = intent.at.after(spec.timeout);
    let premature = |due: Timestamp| context.at < due;
    // A result counts only if it arrived before the session gave up on the
    // tool: an idempotent one at its timeout or cancellation, an unsafe one
    // once its grace period after either ran out.
    let late = match safety {
        Safety::Idempotent => context.cancelled.is_some() || context.at >= timeout,
        Safety::Unsafe => {
            let grace = spec.cancel_grace;
            let after_cancel = context
                .cancelled
                .map(|(_, cancelled_at)| cancelled_at.after(grace));
            let hard =
                after_cancel.map_or(timeout.after(grace), |due| due.min(timeout.after(grace)));
            context.at >= hard
        }
    };
    let check_reason = |reason: &StopReason| match reason {
        StopReason::TurnCancelled if context.cancelled.is_none() => {
            Err(Violation::WrongStopReason(effect))
        }
        StopReason::TimedOut if premature(timeout) => Err(Violation::PrematureTimeout(effect)),
        StopReason::TurnCancelled | StopReason::TimedOut => Ok(()),
    };
    match outcome {
        ToolOutcome::Returned { output, is_error } => {
            let faithful = env.delivered(effect).iter().any(|input| {
                matches!(input, Input::ToolReturned { output: given, is_error: given_error, .. }
                    if given == output && given_error == is_error)
            });
            if !faithful {
                return Err(Violation::UnfaithfulOutcome(effect));
            }
            if late {
                return Err(Violation::LateResult(effect));
            }
        }
        ToolOutcome::NotRun { reason } => {
            if env.started(effect) {
                return Err(Violation::UntruthfulNotRun(effect));
            }
            check_reason(reason)?;
        }
        ToolOutcome::Stopped { reason } => {
            if !env.stopped(effect) {
                return Err(Violation::UnconfirmedStop(effect));
            }
            if late {
                return Err(Violation::LateResult(effect));
            }
            check_reason(reason)?;
        }
        ToolOutcome::Unknown {
            cause: UnknownCause::Restart,
        } => {
            if *safety != Safety::Unsafe || !context.restarted {
                return Err(Violation::UnfaithfulOutcome(effect));
            }
        }
        ToolOutcome::Unknown {
            cause: UnknownCause::Abandoned,
        } => {
            let grace = spec.cancel_grace;
            let after_cancel = context
                .cancelled
                .map(|(_, cancelled_at)| cancelled_at.after(grace));
            let earliest = match *safety {
                Safety::Unsafe => {
                    let after_timeout = timeout.after(grace);
                    after_cancel.map_or(after_timeout, |due| due.min(after_timeout))
                }
                Safety::Idempotent => after_cancel.map_or(timeout, |due| due.min(timeout)),
            };
            if premature(earliest) {
                return Err(Violation::PrematureTimeout(effect));
            }
            if *safety == Safety::Unsafe && grace > Millis::ZERO && !env.cancel_sent(effect) {
                return Err(Violation::MissingCancel(effect));
            }
        }
        ToolOutcome::UnknownTool => return Err(Violation::UnfaithfulOutcome(effect)),
    }
    Ok(())
}

/// Nothing of a cancelled turn is dispatched once the cancellation is
/// recorded.
fn check_dispatches(world: &World, scan: &Scan) -> Result<(), Violation> {
    for (effect, intent) in &scan.intents {
        let cancelled = scan
            .turns
            .get(&intent.turn)
            .and_then(|facts| facts.cancelled);
        if let (Some((cancelled_at, _)), Some(dispatched_at)) =
            (cancelled, world.env().dispatched_at(*effect))
            && dispatched_at > cancelled_at
        {
            return Err(Violation::DispatchAfterCancel(*effect));
        }
    }
    Ok(())
}

/// By the session's own clock, nothing it should already have done is
/// left undone.
fn check_nothing_overdue(
    world: &World,
    scan: &Scan,
    produced: &[&Record],
) -> Result<(), Violation> {
    let session = world.session();
    let now = session.now();
    let env = world.env();
    let Some(RecordBody::SessionCreated { model, .. }) =
        produced.first().map(|record| &record.body)
    else {
        return Err(Violation::ReplayDiverged);
    };
    // The earliest moment the session has something to do without input.
    let mut next: Option<Timestamp> = None;
    let mut wake_by = |at: Timestamp| next = Some(next.map_or(at, |next| next.min(at)));
    for (effect, intent) in &scan.intents {
        if scan.resolved.contains(effect) {
            continue;
        }
        let cancelled = scan
            .turns
            .get(&intent.turn)
            .and_then(|facts| facts.cancelled)
            .map(|(_, cancelled_at)| cancelled_at);
        let due = match &intent.tool {
            None => cancelled.unwrap_or_else(|| intent.at.after(model.timeout)),
            Some((_, None)) => intent.at,
            Some((Safety::Idempotent, Some(spec))) => {
                let timeout = intent.at.after(spec.timeout);
                cancelled.map_or(timeout, |at| at.min(timeout))
            }
            Some((Safety::Unsafe, Some(spec))) => {
                let stop = intent.at.after(spec.timeout);
                let ask = cancelled.map_or(stop, |at| at.min(stop));
                if spec.cancel_grace > Millis::ZERO
                    && now >= ask
                    && env.running(*effect)
                    && !env.stopping(*effect)
                {
                    return Err(Violation::MissingCancel(*effect));
                }
                if !env.stopping(*effect) {
                    wake_by(ask);
                }
                ask.after(spec.cancel_grace)
            }
        };
        if now >= due {
            return Err(Violation::Overdue(*effect));
        }
        wake_by(due);
    }
    let mut active = false;
    for (turn, facts) in &scan.turns {
        if facts.ended {
            continue;
        }
        active = true;
        let awaiting_model = scan.intents.iter().any(|(effect, intent)| {
            intent.turn == *turn && intent.tool.is_none() && !scan.resolved.contains(effect)
        });
        if awaiting_model || facts.cancelled.is_some() {
            continue;
        }
        if facts.finished_failing {
            return Err(Violation::TurnNotEnded(*turn));
        }
        if let Some(retry_at) = facts.retry_at {
            // Jitter adds at most a quarter of the backoff.
            let jitter = model
                .retry
                .backoff(facts.failures)
                .get()
                .checked_shr(2)
                .unwrap_or(0);
            let latest = retry_at.after(Millis::new(jitter));
            if now >= latest {
                return Err(Violation::RetryOverdue(*turn));
            }
            wake_by(latest);
        }
    }
    if next.is_some_and(|next| session.deadline().is_none_or(|deadline| deadline > next)) {
        return Err(Violation::LateDeadline);
    }
    if !active && scan.admitted.len() > scan.started.len() {
        return Err(Violation::IdleWithQueue);
    }
    Ok(())
}
