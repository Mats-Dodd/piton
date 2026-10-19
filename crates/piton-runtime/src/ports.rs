//! The interfaces the runtime does its IO through. Every source of
//! nondeterminism a session sees arrives through one of these.

use std::fmt;

use async_trait::async_trait;
use piton_core::JournalEntry;
use piton_domain::{
    EffectId, Json, Millis, ModelRequest, ModelResponse, Offset, Record, SessionId, Timestamp,
    ToolCall,
};
use tokio::sync::watch;

/// Durable, ordered storage for session logs.
#[async_trait]
pub trait LogStore: fmt::Debug + Send + Sync {
    /// The records of `session` from offset `from` onward.
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError>;

    /// Appends `records` atomically if the log currently ends at `expected`.
    ///
    /// An error says nothing about whether the records landed: they may
    /// land now, later, or never. The runtime copes with all three.
    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError>;
}

/// Calls a language model.
#[async_trait]
pub trait ModelProvider: fmt::Debug + Send + Sync {
    /// Makes one call. `effect` identifies it across retries of delivery and
    /// restarts of the session, so it can serve as an idempotency key. The
    /// runtime drops the future if the call is abandoned.
    async fn call(
        &self,
        effect: EffectId,
        request: ModelRequest,
    ) -> Result<ModelResponse, ModelError>;
}

/// Runs tools.
#[async_trait]
pub trait ToolHost: fmt::Debug + Send + Sync {
    /// Runs one tool call. `effect` identifies it across restarts of the
    /// session, so it can serve as an idempotency key.
    ///
    /// `cancel` fires when the session asks the tool to stop. A tool that
    /// stops answers [`ToolCompletion::Stopped`]; one that ignores the
    /// signal is eventually abandoned and its outcome recorded as unknown.
    async fn run(&self, effect: EffectId, call: ToolCall, cancel: CancelSignal) -> ToolCompletion;
}

/// The current time.
pub trait Clock: fmt::Debug + Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Receives every entry of every session's journal: what crossed the
/// boundary of the core, in order. For observing and replaying sessions;
/// never needed for recovery.
pub trait TelemetryJournal: fmt::Debug + Send + Sync {
    /// Called on the session's task; must not block.
    fn record(&self, session: &SessionId, entry: &JournalEntry);
}

/// A failed model call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError {
    pub reason: String,
    /// Whether trying again could succeed, such as after a rate limit or an
    /// overloaded provider.
    pub retryable: bool,
    /// How long the provider asked callers to wait before trying again.
    pub retry_after: Option<Millis>,
}

/// How a tool call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolCompletion {
    /// The tool ran; `is_error` is its own verdict.
    Returned { output: Json, is_error: bool },
    /// The tool stopped, before producing output, because it was asked to.
    Stopped,
}

/// A storage failure. Deliberately opaque: whatever went wrong, the runtime
/// recovers by reading the log back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError {
    reason: String,
}

impl StoreError {
    #[must_use]
    pub const fn new(reason: String) -> Self {
        Self { reason }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "storage failed: {}", self.reason)
    }
}

impl std::error::Error for StoreError {}

/// Fires when the session asks a running tool to stop.
#[derive(Clone, Debug)]
pub struct CancelSignal(watch::Receiver<bool>);

impl CancelSignal {
    /// A signal and the switch that fires it.
    pub(crate) fn new() -> (watch::Sender<bool>, Self) {
        let (switch, signal) = watch::channel(false);
        (switch, Self(signal))
    }

    /// A signal that never fires, for running a tool outside a session.
    #[must_use]
    pub fn never() -> Self {
        Self::new().1
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Completes once the session asks the tool to stop; never, if it
    /// doesn't.
    pub async fn cancelled(&self) {
        let mut signal = self.0.clone();
        if signal.wait_for(|cancelled| *cancelled).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
