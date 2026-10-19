//! Ready-made ports: in-memory storage, clocks and a journal that discards.

use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use piton_core::JournalEntry;
use piton_domain::{Millis, Offset, Record, SessionId, Timestamp};

use crate::{Clock, LogStore, StoreError, TelemetryJournal};

/// Session logs held in memory. For tests, simulations and sessions that
/// need not outlive the process.
#[derive(Debug, Default)]
pub struct MemLogStore {
    logs: Mutex<BTreeMap<SessionId, Vec<Record>>>,
}

impl MemLogStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every record of `session`, or an empty log if it does not exist.
    #[must_use]
    pub fn records(&self, session: &SessionId) -> Vec<Record> {
        self.logs().get(session).cloned().unwrap_or_default()
    }

    /// [`LogStore::append`], without waiting.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if the log does not end at `expected`.
    pub fn append_now(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        let mut logs = self.logs();
        let log = logs.entry(session.clone()).or_default();
        let tail = u64::try_from(log.len()).unwrap_or(u64::MAX);
        let outcome = if tail == expected.get() {
            log.extend(records);
            Ok(())
        } else {
            Err(StoreError::new(format!(
                "expected the log to end at {}, but it ends at {tail}",
                expected.get()
            )))
        };
        drop(logs);
        outcome
    }

    fn logs(&self) -> std::sync::MutexGuard<'_, BTreeMap<SessionId, Vec<Record>>> {
        self.logs.lock().expect("the store is not poisoned")
    }
}

#[async_trait]
impl LogStore for MemLogStore {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        let skip = usize::try_from(from.get()).unwrap_or(usize::MAX);
        self.logs()
            .get(session)
            .map(|log| log.iter().skip(skip).cloned().collect())
            .ok_or_else(|| StoreError::new(format!("no session {}", session.as_str())))
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        self.append_now(session, expected, records)
    }
}

/// The operating system's wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

/// Time as tokio measures it, starting from a chosen timestamp. Follows
/// paused and simulated time, so tests and simulations stay deterministic.
#[derive(Clone, Copy, Debug)]
pub struct TokioClock {
    origin: tokio::time::Instant,
    start: Timestamp,
}

impl TokioClock {
    /// A clock that reads `start` now. Must be called inside a tokio
    /// runtime.
    #[must_use]
    pub fn starting_at(start: Timestamp) -> Self {
        Self {
            origin: tokio::time::Instant::now(),
            start,
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.start.after(Millis::new(elapsed))
    }
}

/// A journal that keeps nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopTelemetryJournal;

impl TelemetryJournal for NoopTelemetryJournal {
    fn record(&self, _session: &SessionId, _entry: &JournalEntry) {}
}
