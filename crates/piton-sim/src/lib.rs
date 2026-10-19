//! Deterministic simulation of `piton-runtime` under faults.
//!
//! One seed drives everything: a turmoil simulation runs the real runtime
//! on a host, against storage, a model and tools that fail in every way the
//! runtime claims to survive, while the host itself crashes and restarts.
//! mad-turmoil pins the process's clocks and entropy to the simulation, so
//! a seed replays to the byte.
//!
//! Every run is then checked three ways:
//!
//! - its journal replays through the model harness (`piton_model::validate`):
//!   the real runtime only did what the model's environment allows, and every
//!   invariant held;
//! - what tools really did matches what the log claims;
//! - once faults stop, every request is admitted and every turn ends.
//!
//! Simulations must run with `--cfg tokio_unstable`, which seeds tokio's
//! own randomness (`just sim`). Link this crate only into simulation
//! binaries: it replaces the process's clock.

mod host;
mod ports;
mod world;

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, OnceLock},
    time::Duration,
};

use mad_turmoil::time::SimClocksGuard;
use piton_core::JournalEntry;
use piton_domain::{
    EffectId, Millis, ModelPolicy, Record, RecordBody, RetryPolicy, Safety, StopReason, Timestamp,
    ToolName, ToolOutcome, ToolSpec,
};
use piton_model::{Divergence, WorldConfig, validate};
use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};

pub use world::ToolRun;

use crate::world::SimWorld;

/// The session's model policy in every simulation.
pub const MODEL_POLICY: ModelPolicy = ModelPolicy {
    timeout: Millis::new(100),
    retry: RetryPolicy {
        max_attempts: 3,
        initial_backoff: Millis::new(10),
        max_backoff: Millis::new(80),
    },
};

/// The session's tools in every simulation.
#[must_use]
pub fn tools() -> Vec<ToolSpec> {
    Vec::from([
        ToolSpec {
            name: ToolName::new("search".to_owned()),
            safety: Safety::Idempotent,
            timeout: Millis::new(50),
            cancel_grace: Millis::ZERO,
        },
        ToolSpec {
            name: ToolName::new("send_email".to_owned()),
            safety: Safety::Unsafe,
            timeout: Millis::new(50),
            cancel_grace: Millis::new(20),
        },
    ])
}

/// What a simulation does.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    /// How many requests the client submits.
    pub requests: u8,
    /// Simulated time between the client's requests.
    pub request_gap_millis: u64,
    /// How long faults and crashes run.
    pub fault_millis: u64,
    /// How long the system then has to finish everything, without faults.
    pub settle_millis: u64,
    /// Chance per millisecond, in thousandths, that the host crashes.
    pub crash_per_mille: u32,
}

impl Plan {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            requests: 6,
            request_gap_millis: 200,
            fault_millis: 3_000,
            settle_millis: 5_000,
            crash_per_mille: 3,
        }
    }
}

/// Everything a simulation produced.
#[derive(Clone, Debug)]
pub struct Run {
    pub seed: u64,
    pub journal: Vec<JournalEntry>,
    /// The log as storage holds it at the end.
    pub log: Vec<Record>,
    /// What tools really did.
    pub truth: BTreeMap<EffectId, ToolRun>,
    /// Whether everything finished once faults stopped.
    pub settled: bool,
    pub crashes: u32,
    /// How many appends landed after the runtime gave up on them.
    pub late_landed: u32,
}

/// How a simulation went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Turmoil reported an error: a host's software failed or panicked.
    Simulation(String),
    /// Work was left unfinished long after faults stopped.
    Unsettled,
    /// The runtime's journal does not replay through the model.
    Diverged(Divergence),
    /// The log claims something about a tool that did not happen.
    Untruthful(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Failure {}

/// Runs the simulation for `seed`.
///
/// # Errors
///
/// Returns [`Failure::Simulation`] if turmoil reports an error.
pub fn run(seed: u64, plan: Plan) -> Result<Run, Failure> {
    pin_process_entropy();
    let world = Arc::new(SimWorld::new(seed, plan));
    let total = plan.fault_millis.saturating_add(plan.settle_millis);
    let mut sim = turmoil::Builder::new()
        .rng_seed(seed)
        .tick_duration(Duration::from_millis(1))
        .simulation_duration(Duration::from_millis(total.saturating_add(1_000)))
        .build();
    let host_world = Arc::clone(&world);
    sim.host("runtime", move || host::run(Arc::clone(&host_world)));

    let mut chaos = StdRng::seed_from_u64(seed.rotate_left(17));
    let mut down_until = None;
    let mut crashes: u32 = 0;
    let mut settled = false;
    loop {
        let now = millis(sim.elapsed());
        if now >= total {
            break;
        }
        let faulty = now < plan.fault_millis;
        if !faulty && world.faults_on() {
            world.stop_faults();
        }
        match down_until {
            Some(until) if now >= until || !faulty => {
                sim.bounce("runtime");
                down_until = None;
            }
            None if faulty && chaos.random_ratio(plan.crash_per_mille, 1000) => {
                sim.crash("runtime");
                crashes = crashes.saturating_add(1);
                let downtime: u64 = chaos.random_range(0..50);
                down_until = Some(now.saturating_add(downtime));
            }
            Some(_) | None => {}
        }
        sim.step()
            .map_err(|error| Failure::Simulation(error.to_string()))?;
        world.land_late_appends(Timestamp::from_millis(millis(sim.elapsed())));
        if !faulty && is_settled(&world) {
            settled = true;
            break;
        }
    }
    Ok(Run {
        seed,
        journal: world.journal(),
        log: world.store.records(&world.session),
        truth: world.truth(),
        settled,
        crashes,
        late_landed: world.late_landed(),
    })
}

/// Checks a run: it settled, its journal replays through the model, and its
/// log tells the truth about every tool.
///
/// # Errors
///
/// Returns the first check the run fails.
pub fn check(run: &Run) -> Result<(), Failure> {
    validate(&WorldConfig::replaying(tools(), MODEL_POLICY), &run.journal)
        .map_err(Failure::Diverged)?;
    check_truth(run)?;
    if run.settled {
        Ok(())
    } else {
        Err(Failure::Unsettled)
    }
}

/// What tools really did matches what the log says they did.
fn check_truth(run: &Run) -> Result<(), Failure> {
    let mut safety = BTreeMap::new();
    for record in &run.log {
        let effect = EffectId::new(record.offset);
        match &record.body {
            RecordBody::ToolIntent {
                safety: declared, ..
            } => {
                safety.insert(effect, *declared);
                let starts = run.truth.get(&effect).map_or(0, |truth| truth.starts);
                if *declared == Safety::Unsafe && starts > 1 {
                    return Err(Failure::Untruthful(format!(
                        "unsafe tool {} ran {starts} times",
                        effect.intent().get()
                    )));
                }
            }
            RecordBody::ToolResolved { effect, outcome } => {
                let truth = run.truth.get(effect).cloned().unwrap_or_default();
                let unsafe_tool = safety.get(effect) == Some(&Safety::Unsafe);
                let lie = match outcome {
                    ToolOutcome::NotRun { .. } => truth.starts > 0,
                    ToolOutcome::Stopped { .. } => unsafe_tool && !truth.stopped,
                    ToolOutcome::Returned { output, .. } => !truth.outputs.contains(output),
                    ToolOutcome::Unknown { .. } | ToolOutcome::UnknownTool => false,
                };
                if lie {
                    return Err(Failure::Untruthful(format!(
                        "tool {} recorded as {outcome:?}, but really {truth:?}",
                        effect.intent().get()
                    )));
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
fn is_settled(world: &SimWorld) -> bool {
    let log = world.store.records(&world.session);
    let mut admitted: u32 = 0;
    let mut turns: i64 = 0;
    let mut open: i64 = 0;
    for record in &log {
        match record.body {
            RecordBody::InputAdmitted { .. } => admitted = admitted.saturating_add(1),
            RecordBody::TurnStarted { .. } => turns = turns.saturating_add(1),
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
    let started = log
        .iter()
        .filter(|record| matches!(record.body, RecordBody::TurnStarted { .. }))
        .count();
    admitted == u32::from(world.plan.requests)
        && usize::try_from(admitted).is_ok_and(|admitted| admitted == started)
        && turns == 0
        && open == 0
}

/// How a run exercised the runtime, for checking that the faults bite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub recoveries: u32,
    pub crashes: u32,
    pub late_landed: u32,
    pub retries: u32,
    pub timeouts: u32,
    pub stopped: u32,
    pub abandoned: u32,
    pub not_run: u32,
    pub restarted: u32,
}

impl Coverage {
    /// What `run` exercised.
    #[must_use]
    pub fn of(run: &Run) -> Self {
        let mut coverage = Self {
            crashes: run.crashes,
            late_landed: run.late_landed,
            ..Self::default()
        };
        let creations: u32 = 1;
        coverage.recoveries = u32::try_from(
            run.journal
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::Recovered { .. }))
                .count(),
        )
        .unwrap_or(u32::MAX)
        .saturating_sub(creations);
        for record in &run.log {
            let bump = |counter: &mut u32| *counter = counter.saturating_add(1);
            match &record.body {
                RecordBody::ModelResolved {
                    outcome:
                        piton_domain::ModelOutcome::Failed {
                            retryable: true, ..
                        },
                    ..
                } => bump(&mut coverage.retries),
                RecordBody::ToolResolved { outcome, .. } => match outcome {
                    ToolOutcome::Stopped {
                        reason: StopReason::TimedOut,
                    }
                    | ToolOutcome::NotRun {
                        reason: StopReason::TimedOut,
                    } => bump(&mut coverage.timeouts),
                    ToolOutcome::Stopped {
                        reason: StopReason::TurnCancelled,
                    } => bump(&mut coverage.stopped),
                    ToolOutcome::NotRun {
                        reason: StopReason::TurnCancelled,
                    } => bump(&mut coverage.not_run),
                    ToolOutcome::Unknown {
                        cause: piton_domain::UnknownCause::Abandoned,
                    } => bump(&mut coverage.abandoned),
                    ToolOutcome::Unknown {
                        cause: piton_domain::UnknownCause::Restart,
                    } => bump(&mut coverage.restarted),
                    ToolOutcome::Returned { .. } | ToolOutcome::UnknownTool => {}
                },
                RecordBody::SessionCreated { .. }
                | RecordBody::SessionResumed
                | RecordBody::InputAdmitted { .. }
                | RecordBody::TurnStarted { .. }
                | RecordBody::ModelIntent { .. }
                | RecordBody::ModelResolved { .. }
                | RecordBody::ToolIntent { .. }
                | RecordBody::TurnCancelled { .. }
                | RecordBody::TurnEnded { .. } => {}
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
            stopped: self.stopped.saturating_add(other.stopped),
            abandoned: self.abandoned.saturating_add(other.abandoned),
            not_run: self.not_run.saturating_add(other.not_run),
            restarted: self.restarted.saturating_add(other.restarted),
        }
    }
}

/// Points the process's clocks and entropy at the simulation, once.
fn pin_process_entropy() {
    static PINNED: OnceLock<SimClocksGuard> = OnceLock::new();
    PINNED.get_or_init(|| {
        mad_turmoil::rand::set_rng(StdRng::seed_from_u64(0));
        SimClocksGuard::init()
    });
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
