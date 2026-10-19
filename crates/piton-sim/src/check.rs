//! What a simulation run must satisfy, and what it exercised.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use piton_core::{Effect, Input, JournalEntry, Submission};
use piton_domain::{
    EffectId, ModelOutcome, RecordBody, Safety, StopReason, ToolOutcome, UnknownCause,
};
use piton_model::{Divergence, WorldConfig, validate};
use piton_runtime::ModelError;

use crate::{
    MODEL_POLICY, Run, tools,
    world::{Happened, SimWorld},
};

/// How a simulation went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The simulation stopped early: turmoil reported an error, or
    /// something panicked.
    Aborted(String),
    /// Work was left unfinished long after faults stopped, or the runtime
    /// did not fall quiet.
    Unsettled,
    /// The runtime's journal does not replay through the model.
    Diverged(Divergence),
    /// The journal or the log claims something a port did not do.
    Untruthful(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Failure {}

/// Checks a run: it finished, its journal replays through the model, and
/// both the journal and the log are true to what the ports did.
///
/// # Errors
///
/// Returns the first check the run fails.
pub fn check(run: &Run) -> Result<(), Failure> {
    if let Some(reason) = &run.aborted {
        return Err(Failure::Aborted(reason.clone()));
    }
    validate(&WorldConfig::replaying(tools(), MODEL_POLICY), &run.journal)
        .map_err(Failure::Diverged)?;
    check_journal_against_ports(run)?;
    check_runs(run)?;
    check_log_against_ports(run)?;
    if run.settled {
        Ok(())
    } else {
        Err(Failure::Unsettled)
    }
}

const fn untruthful(message: String) -> Result<(), Failure> {
    Err(Failure::Untruthful(message))
}

/// Every result the runtime handed the core is exactly one a port produced
/// before then: the runtime neither invents nor alters results.
fn check_journal_against_ports(run: &Run) -> Result<(), Failure> {
    for (index, entry) in run.journal.iter().enumerate() {
        let JournalEntry::Handled { input, .. } = entry else {
            continue;
        };
        let produced = |effect: &EffectId, matches: &dyn Fn(&Happened) -> bool| {
            run.truth.iter().any(|truth| {
                truth.effect == *effect && truth.seen <= index && matches(&truth.event)
            })
        };
        let true_to_ports = match input {
            Input::ModelResponded { effect, response } => produced(
                effect,
                &|event| matches!(event, Happened::ModelReplied(Ok(given)) if given == response),
            ),
            Input::ModelFailed {
                effect,
                reason,
                retryable,
                retry_after,
            } => {
                let expected = ModelError {
                    reason: reason.clone(),
                    retryable: *retryable,
                    retry_after: *retry_after,
                };
                produced(
                    effect,
                    &|event| matches!(event, Happened::ModelReplied(Err(given)) if *given == expected),
                )
            }
            Input::ToolReturned {
                effect,
                output,
                is_error,
            } => produced(effect, &|event| {
                matches!(event, Happened::ToolReturned { output: given, is_error: given_error }
                    if given == output && given_error == is_error)
            }),
            Input::EffectCancelled { effect } => {
                produced(effect, &|event| matches!(event, Happened::ToolStopped))
            }
            Input::Appended { .. }
            | Input::Submit { .. }
            | Input::CancelTurn { .. }
            | Input::Tick => true,
        };
        if !true_to_ports {
            return untruthful(format!(
                "journal entry {index} hands the core {input:?}, which no port produced"
            ));
        }
    }
    Ok(())
}

/// A tool runs only while a dispatch of it is live, from the entry that
/// dispatched it until its result, its abandonment or the next recovery,
/// and starts at most once per dispatch. Anything else is a task the
/// runtime failed to stop.
fn check_runs(run: &Run) -> Result<(), Failure> {
    let journal = &run.journal;
    let mut live: BTreeMap<EffectId, Vec<(usize, usize)>> = BTreeMap::new();
    for (start, entry) in journal.iter().enumerate() {
        for submission in submissions(entry) {
            let Submission::Dispatch {
                effect: Effect::RunTool { id, .. },
            } = submission
            else {
                continue;
            };
            let end = journal
                .iter()
                .enumerate()
                .skip(start.saturating_add(1))
                .find(|(_, later)| ends_run(later, *id))
                .map_or(journal.len(), |(end, _)| end);
            live.entry(*id).or_default().push((start, end));
        }
    }
    let mut starts: BTreeMap<(EffectId, usize), u32> = BTreeMap::new();
    for truth in &run.truth {
        if matches!(truth.event, Happened::ModelReplied(_)) {
            continue;
        }
        let Some(&(start, _)) = live.get(&truth.effect).and_then(|runs| {
            runs.iter()
                .find(|(start, end)| *start < truth.seen && truth.seen <= *end)
        }) else {
            return untruthful(format!(
                "tool {} did {:?} outside any live dispatch",
                truth.effect.intent().get(),
                truth.event
            ));
        };
        if truth.event == Happened::ToolStarted {
            let count = starts.entry((truth.effect, start)).or_default();
            *count = count.saturating_add(1);
            if *count > 1 {
                return untruthful(format!(
                    "tool {} started twice for one dispatch",
                    truth.effect.intent().get()
                ));
            }
        }
    }
    Ok(())
}

/// Whether `entry` ends a run of `effect`: its result arrived, the session
/// abandoned it, or the session recovered and stopped everything.
fn ends_run(entry: &JournalEntry, effect: EffectId) -> bool {
    match entry {
        JournalEntry::Recovered { .. } => true,
        JournalEntry::Handled {
            input, submissions, ..
        } => {
            let completed = matches!(input,
                Input::ToolReturned { effect: done, .. } | Input::EffectCancelled { effect: done }
                if *done == effect);
            let abandoned = submissions.iter().any(
                |submission| matches!(submission, Submission::Abandon { effect: dropped } if *dropped == effect),
            );
            completed || abandoned
        }
    }
}

fn submissions(entry: &JournalEntry) -> &[Submission] {
    match entry {
        JournalEntry::Handled { submissions, .. } | JournalEntry::Recovered { submissions, .. } => {
            submissions
        }
    }
}

/// What the log finally says about each tool matches what it really did,
/// and no unsafe tool ever started twice.
fn check_log_against_ports(run: &Run) -> Result<(), Failure> {
    let happened = |effect: EffectId, matches: &dyn Fn(&Happened) -> bool| {
        run.truth
            .iter()
            .filter(|truth| truth.effect == effect && matches(&truth.event))
            .count()
    };
    let mut safety = BTreeMap::new();
    for record in &run.log {
        match &record.body {
            RecordBody::ToolIntent {
                safety: declared, ..
            } => {
                let effect = EffectId::new(record.offset);
                safety.insert(effect, *declared);
                let starts = happened(effect, &|event| *event == Happened::ToolStarted);
                if *declared == Safety::Unsafe && starts > 1 {
                    return untruthful(format!(
                        "unsafe tool {} ran {starts} times",
                        effect.intent().get()
                    ));
                }
            }
            RecordBody::ToolResolved { effect, outcome } => {
                let lie = match outcome {
                    ToolOutcome::NotRun { .. } => {
                        happened(*effect, &|event| *event == Happened::ToolStarted) > 0
                    }
                    ToolOutcome::Stopped { .. } => {
                        happened(*effect, &|event| *event == Happened::ToolStopped) == 0
                    }
                    ToolOutcome::Returned { output, is_error } => {
                        happened(*effect, &|event| {
                            matches!(event, Happened::ToolReturned { output: given, is_error: given_error }
                                if given == output && given_error == is_error)
                        }) == 0
                    }
                    ToolOutcome::Unknown { .. } | ToolOutcome::UnknownTool => false,
                };
                if lie {
                    return untruthful(format!(
                        "tool {} recorded as {outcome:?}, which it did not do",
                        effect.intent().get()
                    ));
                }
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::SessionResumed
            | RecordBody::InputAdmitted { .. }
            | RecordBody::TurnStarted { .. }
            | RecordBody::ModelIntent { .. }
            | RecordBody::ModelResolved { .. }
            | RecordBody::TurnCancelled { .. }
            | RecordBody::TurnEnded { .. } => {}
        }
    }
    Ok(())
}

/// Every planned request is durably admitted, every turn has ended and
/// every effect is resolved.
pub(crate) fn is_settled(world: &SimWorld) -> bool {
    let log = world.store.records(&world.session);
    let mut admitted: u32 = 0;
    let mut started: u32 = 0;
    let mut turns: i64 = 0;
    let mut open: i64 = 0;
    for record in &log {
        match record.body {
            RecordBody::InputAdmitted { .. } => admitted = admitted.saturating_add(1),
            RecordBody::TurnStarted { .. } => {
                started = started.saturating_add(1);
                turns = turns.saturating_add(1);
            }
            RecordBody::TurnEnded { .. } => turns = turns.saturating_sub(1),
            RecordBody::ModelIntent { .. } | RecordBody::ToolIntent { .. } => {
                open = open.saturating_add(1);
            }
            RecordBody::ModelResolved { .. } | RecordBody::ToolResolved { .. } => {
                open = open.saturating_sub(1);
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::SessionResumed
            | RecordBody::TurnCancelled { .. } => {}
        }
    }
    admitted == u32::from(world.plan.requests) && started == admitted && turns == 0 && open == 0
}

/// How a run exercised the runtime, for checking that the faults bite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub recoveries: u32,
    pub crashes: u32,
    pub late_landed: u32,
    pub retries: u32,
    pub timeouts: u32,
    /// Unsafe tools that confirmed they stopped.
    pub confirmed_stops: u32,
    pub abandoned: u32,
    pub not_run: u32,
    pub restarted: u32,
    /// Results that arrived after their effect's time ran out.
    pub lost_races: u32,
}

impl Coverage {
    /// What `run` exercised.
    #[must_use]
    pub fn of(run: &Run) -> Self {
        let bump = |counter: &mut u32| *counter = counter.saturating_add(1);
        let mut coverage = Self {
            crashes: run.crashes,
            late_landed: run.late_landed,
            ..Self::default()
        };
        let recovered = run
            .journal
            .iter()
            .filter(|entry| matches!(entry, JournalEntry::Recovered { .. }))
            .count();
        // The first is the session's creation.
        coverage.recoveries = u32::try_from(recovered.saturating_sub(1)).unwrap_or(u32::MAX);
        let mut safety = BTreeMap::new();
        let mut by_timer = BTreeSet::new();
        for record in &run.log {
            match &record.body {
                RecordBody::ToolIntent { safety: given, .. } => {
                    safety.insert(EffectId::new(record.offset), *given);
                }
                RecordBody::ModelResolved {
                    effect,
                    outcome:
                        ModelOutcome::Failed {
                            retryable, reason, ..
                        },
                } => {
                    if *retryable {
                        bump(&mut coverage.retries);
                    }
                    if reason == "timed out" {
                        by_timer.insert(*effect);
                    }
                }
                RecordBody::ToolResolved { effect, outcome } => {
                    tally(&mut coverage, &safety, *effect, outcome);
                    if matches!(
                        outcome,
                        ToolOutcome::Unknown {
                            cause: UnknownCause::Abandoned
                        }
                    ) {
                        by_timer.insert(*effect);
                    }
                }
                RecordBody::SessionCreated { .. }
                | RecordBody::SessionResumed
                | RecordBody::InputAdmitted { .. }
                | RecordBody::TurnStarted { .. }
                | RecordBody::ModelIntent { .. }
                | RecordBody::ModelResolved { .. }
                | RecordBody::TurnCancelled { .. }
                | RecordBody::TurnEnded { .. } => {}
            }
        }
        for entry in &run.journal {
            if let JournalEntry::Handled {
                input:
                    Input::ModelResponded { effect, .. }
                    | Input::ModelFailed { effect, .. }
                    | Input::ToolReturned { effect, .. },
                ..
            } = entry
                && by_timer.contains(effect)
            {
                bump(&mut coverage.lost_races);
            }
        }
        coverage
    }

    /// The sum of two coverages.
    #[must_use]
    pub const fn add(self, other: Self) -> Self {
        Self {
            recoveries: self.recoveries.saturating_add(other.recoveries),
            crashes: self.crashes.saturating_add(other.crashes),
            late_landed: self.late_landed.saturating_add(other.late_landed),
            retries: self.retries.saturating_add(other.retries),
            timeouts: self.timeouts.saturating_add(other.timeouts),
            confirmed_stops: self.confirmed_stops.saturating_add(other.confirmed_stops),
            abandoned: self.abandoned.saturating_add(other.abandoned),
            not_run: self.not_run.saturating_add(other.not_run),
            restarted: self.restarted.saturating_add(other.restarted),
            lost_races: self.lost_races.saturating_add(other.lost_races),
        }
    }

    /// Every counter, named, for asserting each is reached.
    #[must_use]
    pub const fn counters(&self) -> [(&'static str, u32); 10] {
        [
            ("recoveries", self.recoveries),
            ("crashes", self.crashes),
            ("late appends landing", self.late_landed),
            ("retries", self.retries),
            ("timeouts", self.timeouts),
            ("unsafe tools confirming a stop", self.confirmed_stops),
            ("abandoned tools", self.abandoned),
            ("tools never run", self.not_run),
            ("tools interrupted by a restart", self.restarted),
            ("results losing to their timers", self.lost_races),
        ]
    }
}

fn tally(
    coverage: &mut Coverage,
    safety: &BTreeMap<EffectId, Safety>,
    effect: EffectId,
    outcome: &ToolOutcome,
) {
    let bump = |counter: &mut u32| *counter = counter.saturating_add(1);
    match outcome {
        ToolOutcome::Stopped { reason } => {
            if *reason == StopReason::TimedOut {
                bump(&mut coverage.timeouts);
            }
            if safety.get(&effect) == Some(&Safety::Unsafe) {
                bump(&mut coverage.confirmed_stops);
            }
        }
        ToolOutcome::NotRun { reason } => {
            if *reason == StopReason::TimedOut {
                bump(&mut coverage.timeouts);
            } else {
                bump(&mut coverage.not_run);
            }
        }
        ToolOutcome::Unknown {
            cause: UnknownCause::Abandoned,
        } => bump(&mut coverage.abandoned),
        ToolOutcome::Unknown {
            cause: UnknownCause::Restart,
        } => bump(&mut coverage.restarted),
        ToolOutcome::Returned { .. } | ToolOutcome::UnknownTool => {}
    }
}
