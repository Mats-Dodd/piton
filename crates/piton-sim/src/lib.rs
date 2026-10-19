//! Deterministic simulation of `piton-runtime` under faults.
//!
//! One seed drives everything: a turmoil simulation runs the real runtime
//! on a host, against storage, a model and tools that fail in every way the
//! runtime claims to survive, while the host crashes and restarts and its
//! clock jumps. mad-turmoil pins the process's clocks and entropy to the
//! simulation, so a seed replays to the byte.
//!
//! Every run is then checked ([`check`]):
//!
//! - its journal replays through the model harness (`piton_model::validate`):
//!   the real runtime only did what the model's environment allows, and every
//!   invariant held;
//! - every result in the journal is exactly what a port produced, and no
//!   tool ran more often than it was dispatched or after its run ended;
//! - once faults stop, every request is admitted, every turn ends, and the
//!   runtime falls quiet.
//!
//! Simulations must run with `--cfg tokio_unstable`, which seeds tokio's
//! own randomness (`just sim`). Link this crate only into simulation
//! binaries: it replaces the process's clock.

mod check;
mod host;
mod ports;
mod world;

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, OnceLock},
    time::Duration,
};

use mad_turmoil::time::SimClocksGuard;
use piton_core::JournalEntry;
use piton_domain::{
    Millis, ModelPolicy, Record, RetryPolicy, Safety, Timestamp, ToolName, ToolSpec,
};
use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};

pub use check::{Coverage, Failure, check};
pub use world::{Happened, Truth};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// How many requests the client submits.
    pub requests: u8,
    /// Simulated time between the client's requests.
    pub request_gap_millis: u64,
    /// How long faults and crashes run.
    pub fault_millis: u64,
    /// How long the system then has to finish everything, without faults.
    pub settle_millis: u64,
    /// How long the runtime must then stay quiet.
    pub quiet_millis: u64,
    /// Chance per millisecond, in thousandths, that the host crashes.
    pub crash_per_mille: u32,
    /// Chance per millisecond, in thousandths, that the clock jumps.
    pub skew_per_mille: u32,
}

impl Plan {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            requests: 6,
            request_gap_millis: 200,
            fault_millis: 3_000,
            settle_millis: 5_000,
            quiet_millis: 300,
            crash_per_mille: 3,
            skew_per_mille: 5,
        }
    }
}

/// Everything a simulation produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub seed: u64,
    pub journal: Vec<JournalEntry>,
    /// The log as storage holds it at the end.
    pub log: Vec<Record>,
    /// What the ports really did, in order.
    pub truth: Vec<Truth>,
    /// Whether everything finished once faults stopped, and then stayed
    /// quiet.
    pub settled: bool,
    pub crashes: u32,
    /// How many appends landed after the runtime gave up on them.
    pub late_landed: u32,
    /// Why the simulation itself stopped early: an error or a panic.
    pub aborted: Option<String>,
}

/// Runs the simulation for `seed`. A panic or simulation error ends the run
/// early and is reported in [`Run::aborted`], with everything gathered so
/// far.
#[must_use]
pub fn run(seed: u64, plan: Plan) -> Run {
    pin_process_entropy();
    let world = Arc::new(SimWorld::new(seed, plan));
    let mut crashes: u32 = 0;
    let driven = catch_unwind(AssertUnwindSafe(|| drive(seed, plan, &world, &mut crashes)));
    let (settled, aborted) = match driven {
        Ok(Ok(settled)) => (settled, None),
        Ok(Err(error)) => (false, Some(error)),
        Err(panic) => (false, Some(panic_message(panic.as_ref()))),
    };
    Run {
        seed,
        journal: world.journal(),
        log: world.store.records(&world.session),
        truth: world.truth(),
        settled,
        crashes,
        late_landed: world.late_landed(),
        aborted,
    }
}

/// Steps the simulation through its fault phase and then lets it settle,
/// returning whether everything finished and stayed quiet.
fn drive(seed: u64, plan: Plan, world: &Arc<SimWorld>, crashes: &mut u32) -> Result<bool, String> {
    let total = plan
        .fault_millis
        .saturating_add(plan.settle_millis)
        .saturating_add(plan.quiet_millis);
    let mut sim = turmoil::Builder::new()
        .rng_seed(seed)
        .tick_duration(Duration::from_millis(1))
        .simulation_duration(Duration::from_millis(total.saturating_add(1_000)))
        .build();
    let host_world = Arc::clone(world);
    sim.host("runtime", move || host::run(Arc::clone(&host_world)));

    let mut chaos = StdRng::seed_from_u64(seed.rotate_left(17));
    let mut down_until = None;
    let mut quiet_from: Option<(u64, usize)> = None;
    loop {
        let now = millis(sim.elapsed());
        if now >= total {
            return Ok(false);
        }
        let faulty = now < plan.fault_millis;
        if !faulty && world.faults_on() {
            world.stop_faults();
            world.set_skew(0);
        }
        if faulty && chaos.random_ratio(plan.skew_per_mille, 1000) {
            world.set_skew(chaos.random_range(0..60));
        }
        match down_until {
            Some(until) if now >= until || !faulty => {
                sim.bounce("runtime");
                down_until = None;
            }
            None if faulty && chaos.random_ratio(plan.crash_per_mille, 1000) => {
                sim.crash("runtime");
                *crashes = crashes.saturating_add(1);
                let downtime: u64 = chaos.random_range(0..50);
                down_until = Some(now.saturating_add(downtime));
            }
            Some(_) | None => {}
        }
        sim.step().map_err(|error| error.to_string())?;
        world.land_late_appends(Timestamp::from_millis(millis(sim.elapsed())));
        if faulty {
            continue;
        }
        // Once settled, the runtime must fall quiet: no journal entries for
        // a while. Each new entry, such as an append still finishing,
        // restarts the wait; a runtime that never stops, such as one stuck
        // recovering, runs out of time.
        let progress = world.journal().len();
        match quiet_from {
            Some((since, entries)) if entries == progress => {
                if now.saturating_sub(since) >= plan.quiet_millis {
                    return Ok(check::is_settled(world));
                }
            }
            Some(_) | None => {
                quiet_from = check::is_settled(world).then_some((now, progress));
            }
        }
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "a panic without a message".to_owned())
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
