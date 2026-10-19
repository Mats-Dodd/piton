//! Everything in a simulation that outlives the runtime's host: storage,
//! the seeded source of faults, the journal, and the truth about what tools
//! really did.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use piton_core::JournalEntry;
use piton_domain::{EffectId, Json, Offset, Record, RequestId, SessionId, Timestamp, TurnId};
use piton_runtime::MemLogStore;
use rand::{RngExt as _, SeedableRng as _, rngs::StdRng};

use crate::Plan;

/// What a tool really did, whatever the log says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolRun {
    /// How many times the tool host started it, across every life.
    pub starts: u32,
    /// Everything it returned.
    pub outputs: Vec<Json>,
    /// Whether it stopped because it was asked to.
    pub stopped: bool,
}

/// An append storage will apply later, after the runtime gave up on it.
#[derive(Debug)]
struct LateAppend {
    expected: Offset,
    records: Vec<Record>,
    due: Timestamp,
}

/// What the workload has seen in the durable log.
#[derive(Debug, Default)]
struct Seen {
    admitted: BTreeSet<RequestId>,
    active_turn: Option<TurnId>,
}

#[derive(Debug)]
pub(crate) struct SimWorld {
    pub(crate) session: SessionId,
    pub(crate) plan: Plan,
    pub(crate) store: MemLogStore,
    rng: Mutex<StdRng>,
    faults: AtomicBool,
    journal: Mutex<Vec<JournalEntry>>,
    truth: Mutex<BTreeMap<EffectId, ToolRun>>,
    late: Mutex<Vec<LateAppend>>,
    late_landed: Mutex<u32>,
    seen: Mutex<Seen>,
}

impl SimWorld {
    pub(crate) fn new(seed: u64, plan: Plan) -> Self {
        Self {
            session: SessionId::new("simulated".to_owned()),
            plan,
            store: MemLogStore::new(),
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
            faults: AtomicBool::new(true),
            journal: Mutex::new(Vec::new()),
            truth: Mutex::new(BTreeMap::new()),
            late: Mutex::new(Vec::new()),
            late_landed: Mutex::new(0),
            seen: Mutex::new(Seen::default()),
        }
    }

    /// Whether a fault happens: with probability `per_mille` / 1000 while
    /// faults are on, never once they are off.
    pub(crate) fn fault(&self, per_mille: u32) -> bool {
        self.faults_on() && self.chance(per_mille)
    }

    /// True with probability `per_mille` / 1000, faults or not.
    pub(crate) fn chance(&self, per_mille: u32) -> bool {
        lock(&self.rng).random_ratio(per_mille, 1000)
    }

    /// A number in `low..high`.
    pub(crate) fn pick(&self, low: u64, high: u64) -> u64 {
        lock(&self.rng).random_range(low..high)
    }

    pub(crate) fn faults_on(&self) -> bool {
        self.faults.load(Ordering::SeqCst)
    }

    pub(crate) fn stop_faults(&self) {
        self.faults.store(false, Ordering::SeqCst);
    }

    pub(crate) fn record(&self, entry: &JournalEntry) {
        lock(&self.journal).push(entry.clone());
    }

    pub(crate) fn journal(&self) -> Vec<JournalEntry> {
        lock(&self.journal).clone()
    }

    pub(crate) fn tool_started(&self, effect: EffectId) {
        let mut truth = lock(&self.truth);
        let run = truth.entry(effect).or_default();
        run.starts = run.starts.saturating_add(1);
        drop(truth);
    }

    pub(crate) fn tool_returned(&self, effect: EffectId, output: Json) {
        lock(&self.truth)
            .entry(effect)
            .or_default()
            .outputs
            .push(output);
    }

    pub(crate) fn tool_stopped(&self, effect: EffectId) {
        lock(&self.truth).entry(effect).or_default().stopped = true;
    }

    pub(crate) fn truth(&self) -> BTreeMap<EffectId, ToolRun> {
        lock(&self.truth).clone()
    }

    /// Storage will try to apply `records` at `expected` once `due` passes.
    pub(crate) fn delay_append(&self, expected: Offset, records: Vec<Record>, due: Timestamp) {
        lock(&self.late).push(LateAppend {
            expected,
            records,
            due,
        });
    }

    /// Applies every late append that is due. Each lands only if the log
    /// still ends where it expected.
    pub(crate) fn land_late_appends(&self, now: Timestamp) {
        let due: Vec<LateAppend> = {
            let mut late = lock(&self.late);
            let (due, pending) = std::mem::take(&mut *late)
                .into_iter()
                .partition(|append| append.due <= now);
            *late = pending;
            due
        };
        for append in due {
            let landed = self
                .store
                .append_now(&self.session, append.expected, append.records)
                .is_ok();
            if landed {
                let mut count = lock(&self.late_landed);
                *count = count.saturating_add(1);
            }
        }
    }

    /// How many late appends landed.
    pub(crate) fn late_landed(&self) -> u32 {
        *lock(&self.late_landed)
    }

    pub(crate) fn saw_admitted(&self, request_id: RequestId) {
        lock(&self.seen).admitted.insert(request_id);
    }

    pub(crate) fn saw_turn(&self, turn: Option<TurnId>) {
        lock(&self.seen).active_turn = turn;
    }

    pub(crate) fn has_seen_admitted(&self, request_id: &RequestId) -> bool {
        lock(&self.seen).admitted.contains(request_id)
    }

    pub(crate) fn active_turn(&self) -> Option<TurnId> {
        lock(&self.seen).active_turn
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .expect("no simulated task panicked holding a lock")
}
