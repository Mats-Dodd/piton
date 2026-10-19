use std::{collections::VecDeque, fmt, sync::Arc};

use piton_core::Input;
use piton_domain::{
    ModelPolicy, Offset, Record, RecordBody, RequestId, SCHEMA_VERSION, SessionId, ToolSpec, TurnId,
};
use tokio::sync::{mpsc, watch};

use crate::{
    Clock, LogStore, ModelProvider, StoreError, TelemetryJournal, ToolHost,
    pump::{Event, OpenError, Pump},
};

/// How many events may wait in a session's queue before senders wait.
const QUEUE_CAPACITY: usize = 256;

/// The IO a runtime does, through the ports it is given.
#[derive(Clone, Debug)]
pub struct Ports {
    pub store: Arc<dyn LogStore>,
    pub model: Arc<dyn ModelProvider>,
    pub tools: Arc<dyn ToolHost>,
    pub clock: Arc<dyn Clock>,
    pub journal: Arc<dyn TelemetryJournal>,
}

/// Runs sessions on the current tokio runtime, one task each.
#[derive(Clone, Debug)]
pub struct Runtime {
    ports: Ports,
}

impl Runtime {
    #[must_use]
    pub const fn new(ports: Ports) -> Self {
        Self { ports }
    }

    /// Creates a session with `tools` and `model` policy and starts it.
    ///
    /// # Errors
    ///
    /// Fails if the session's log cannot be created, for instance because
    /// it already exists, or then cannot be read back.
    pub async fn create(
        &self,
        id: SessionId,
        tools: Vec<ToolSpec>,
        model: ModelPolicy,
    ) -> Result<SessionHandle, OpenError> {
        let created = Record {
            offset: Offset::ZERO,
            at: self.ports.clock.now(),
            body: RecordBody::SessionCreated {
                schema: SCHEMA_VERSION,
                tools,
                model,
            },
        };
        self.ports
            .store
            .append(&id, Offset::ZERO, Vec::from([created]))
            .await
            .map_err(OpenError::Store)?;
        self.open(id).await
    }

    /// Starts an existing session from its log.
    ///
    /// Run one session in one place at a time. Appends are fenced, so a
    /// second copy cannot corrupt the log, but the two would fight over it.
    ///
    /// # Errors
    ///
    /// Fails if the log cannot be read or is not a well-formed session log.
    pub async fn open(&self, id: SessionId) -> Result<SessionHandle, OpenError> {
        let (sender, events) = mpsc::channel(QUEUE_CAPACITY);
        let pump = Pump::open(id.clone(), self.ports.clone(), events, sender.clone()).await?;
        let durable = pump.durable();
        tokio::spawn(pump.run());
        Ok(SessionHandle {
            id,
            events: sender,
            durable,
            store: Arc::clone(&self.ports.store),
        })
    }
}

/// A client's view of a running session. Cheap to clone.
#[derive(Clone, Debug)]
pub struct SessionHandle {
    id: SessionId,
    events: mpsc::Sender<Event>,
    durable: watch::Receiver<Offset>,
    store: Arc<dyn LogStore>,
}

impl SessionHandle {
    #[must_use]
    pub const fn id(&self) -> &SessionId {
        &self.id
    }

    /// Queues `text` for the session. Returns once it is queued, not once it
    /// is admitted; submitting the same `request_id` again is harmless, so a
    /// client unsure whether a submission arrived simply resubmits. Watch
    /// [`SessionHandle::follow`] for `InputAdmitted` to know it did.
    ///
    /// # Errors
    ///
    /// Fails once the session has stopped.
    pub async fn submit(&self, request_id: RequestId, text: String) -> Result<(), SessionClosed> {
        self.send(Input::Submit { request_id, text }).await
    }

    /// Asks the session to cancel `turn`, if it is still running.
    ///
    /// # Errors
    ///
    /// Fails once the session has stopped.
    pub async fn cancel(&self, turn: TurnId) -> Result<(), SessionClosed> {
        self.send(Input::CancelTurn { turn }).await
    }

    /// Follows the durable log from `from`: first the records already
    /// there, then each new one as it becomes durable. Records that are not
    /// yet durable are never shown, so nothing a follower sees is undone.
    #[must_use]
    pub fn follow(&self, from: Offset) -> Follower {
        Follower {
            id: self.id.clone(),
            store: Arc::clone(&self.store),
            durable: self.durable.clone(),
            next: from,
            buffer: VecDeque::new(),
        }
    }

    async fn send(&self, input: Input) -> Result<(), SessionClosed> {
        self.events
            .send(Event::Client(input))
            .await
            .map_err(|_stopped| SessionClosed)
    }
}

/// Reads a session's durable log in order, waiting for new records.
#[derive(Debug)]
pub struct Follower {
    id: SessionId,
    store: Arc<dyn LogStore>,
    durable: watch::Receiver<Offset>,
    next: Offset,
    buffer: VecDeque<Record>,
}

impl Follower {
    /// The next durable record, waiting until there is one.
    ///
    /// # Errors
    ///
    /// Fails if the session stops or its log cannot be read.
    pub async fn next(&mut self) -> Result<Record, FollowError> {
        loop {
            if let Some(record) = self.buffer.pop_front() {
                self.next = record.offset.next();
                return Ok(record);
            }
            let durable = *self.durable.borrow_and_update();
            if self.next < durable {
                let records = self
                    .store
                    .read(&self.id, self.next)
                    .await
                    .map_err(FollowError::Store)?;
                self.buffer.extend(
                    records
                        .into_iter()
                        .take_while(|record| record.offset < durable),
                );
                if self.buffer.is_empty() {
                    return Err(FollowError::Store(StoreError::new(
                        "the log is shorter than its durable length".to_owned(),
                    )));
                }
                continue;
            }
            self.durable
                .changed()
                .await
                .map_err(|_stopped| FollowError::Closed)?;
        }
    }
}

/// The session has stopped and accepts nothing more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionClosed;

impl fmt::Display for SessionClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the session has stopped")
    }
}

impl std::error::Error for SessionClosed {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowError {
    Closed,
    Store(StoreError),
}

impl fmt::Display for FollowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("the session has stopped"),
            Self::Store(error) => write!(formatter, "could not read the log: {error}"),
        }
    }
}

impl std::error::Error for FollowError {}
