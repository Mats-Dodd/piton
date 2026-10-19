//! The task that owns one session and does its IO.
//!
//! The pump makes no decisions; the core makes them all. It turns
//! submissions into IO, IO results into inputs, and feeds inputs to the
//! session one at a time, in the order they reach its queue.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use piton_core::{Effect, Input, JournalEntry, RecoveryError, Session, Submission};
use piton_domain::{EffectId, InvalidPolicy, Json, Offset, Record, SessionId, Timestamp};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::{CancelSignal, ModelError, Ports, StoreError, ToolCompletion};

/// Everything that reaches a session goes through its queue as one of these.
#[derive(Debug)]
pub(crate) enum Event {
    /// IO the pump started finished. `life` is the session's life it was
    /// started in; anything from an earlier life is dropped.
    Completed { life: u64, input: Input },
    /// An append failed and may or may not have landed.
    AppendFailed { life: u64 },
    /// A client's input.
    Client(Input),
}

/// A dispatched effect's task, and the switch that asks a tool to stop.
#[derive(Debug)]
struct Running {
    task: JoinHandle<()>,
    cancel: Option<watch::Sender<bool>>,
}

/// Why a session could not be opened or recovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    Store(StoreError),
    Recovery(RecoveryError),
    /// A different session already has this id.
    AlreadyExists,
    /// The session's policy cannot be followed.
    InvalidPolicy(InvalidPolicy),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "could not read the log: {error}"),
            Self::Recovery(error) => write!(formatter, "could not recover the session: {error}"),
            Self::AlreadyExists => formatter.write_str("a different session already has this id"),
            Self::InvalidPolicy(invalid) => write!(formatter, "invalid session policy: {invalid}"),
        }
    }
}

impl std::error::Error for OpenError {}

#[derive(Debug)]
pub(crate) struct Pump {
    id: SessionId,
    ports: Ports,
    session: Session,
    /// Counts the session's lives in this process; bumped by every recovery.
    life: u64,
    events: mpsc::Receiver<Event>,
    /// Handed to every task the pump starts, so they can report back. Weak,
    /// so that once every client handle is gone the queue closes and the
    /// session stops.
    sender: mpsc::WeakSender<Event>,
    /// Appends that failed in a row; reset by one that lands.
    failed_appends: u32,
    effects: BTreeMap<EffectId, Running>,
    append: Option<JoinHandle<()>>,
    durable: watch::Sender<Offset>,
}

impl Pump {
    /// Reads the session's log and starts its first life in this process.
    pub(crate) async fn open(
        id: SessionId,
        ports: Ports,
        events: mpsc::Receiver<Event>,
        sender: &mpsc::Sender<Event>,
    ) -> Result<Self, OpenError> {
        let (session, log_len, entry) = recover(&id, &ports).await?;
        let mut pump = Self {
            id,
            ports,
            session,
            life: 0,
            events,
            sender: sender.downgrade(),
            failed_appends: 0,
            effects: BTreeMap::new(),
            append: None,
            durable: watch::channel(log_len).0,
        };
        pump.record_and_execute(entry);
        Ok(pump)
    }

    /// Follows the durable length of the log.
    pub(crate) fn durable(&self) -> watch::Receiver<Offset> {
        self.durable.subscribe()
    }

    /// Runs until every client handle is dropped, the log cannot be read
    /// during a recovery, or appends keep failing.
    pub(crate) async fn run(mut self) {
        loop {
            let deadline = self.session.deadline();
            let event = tokio::select! {
                event = self.events.recv() => event,
                () = sleep_until(&self.ports, deadline) => Some(Event::Client(Input::Tick)),
            };
            let Some(event) = event else { return };
            match event {
                Event::Completed { life, input } if life == self.life => {
                    if let Some(effect) = completed_effect(&input) {
                        self.effects.remove(&effect);
                    } else {
                        self.append = None;
                        self.failed_appends = 0;
                    }
                    self.handle(input);
                }
                Event::AppendFailed { life } if life == self.life => {
                    // Storage that keeps failing is not hammered: back off
                    // between recoveries, and give up after a while.
                    self.failed_appends = self.failed_appends.saturating_add(1);
                    if self.failed_appends > MAX_FAILED_APPENDS {
                        return;
                    }
                    tokio::time::sleep(recovery_backoff(self.failed_appends)).await;
                    if self.recover().await.is_err() {
                        return;
                    }
                }
                Event::Client(input) => self.handle(input),
                Event::Completed { .. } | Event::AppendFailed { .. } => {}
            }
        }
    }

    fn handle(&mut self, input: Input) {
        let now = self.ports.clock.now();
        self.session.handle(now, input.clone());
        let entry = JournalEntry::Handled {
            now,
            input,
            submissions: drain(&mut self.session),
        };
        self.record_and_execute(entry);
        let durable = self.session.durable_len();
        self.durable.send_if_modified(|published| {
            let advanced = *published < durable;
            if advanced {
                *published = durable;
            }
            advanced
        });
    }

    /// Starts a new life of the session as if the process had crashed: every
    /// running effect is stopped, everything they or the failed append might
    /// still report is ignored, and the session is rebuilt from the log.
    async fn recover(&mut self) -> Result<(), OpenError> {
        self.stop_everything();
        self.life = self.life.saturating_add(1);
        let (session, log_len, entry) = recover(&self.id, &self.ports).await?;
        self.session = session;
        self.durable.send_replace(log_len);
        self.record_and_execute(entry);
        Ok(())
    }

    fn record_and_execute(&mut self, entry: JournalEntry) {
        self.ports.journal.record(&self.id, &entry);
        let (JournalEntry::Handled { submissions, .. }
        | JournalEntry::Recovered { submissions, .. }) = entry;
        for submission in submissions {
            self.execute(submission);
        }
    }

    fn execute(&mut self, submission: Submission) {
        let life = self.life;
        let sender = self.sender.clone();
        match submission {
            Submission::Append { expected, records } => {
                let store = Arc::clone(&self.ports.store);
                let id = self.id.clone();
                let end = end_of(expected, &records);
                self.append = Some(tokio::spawn(async move {
                    // An append that never answers is as ambiguous as one
                    // that fails.
                    let appended =
                        tokio::time::timeout(APPEND_TIMEOUT, store.append(&id, expected, records))
                            .await;
                    let event = match appended {
                        Ok(Ok(())) => Event::Completed {
                            life,
                            input: Input::Appended { end },
                        },
                        Ok(Err(_)) | Err(_) => Event::AppendFailed { life },
                    };
                    report(&sender, event).await;
                }));
            }
            Submission::Dispatch { effect } => {
                let id = effect.id();
                let running = self.spawn_effect(effect, life, sender);
                self.effects.insert(id, running);
            }
            Submission::Cancel { effect } => {
                if let Some(cancel) = self
                    .effects
                    .get(&effect)
                    .and_then(|running| running.cancel.as_ref())
                {
                    cancel.send_replace(true);
                }
            }
            Submission::Abandon { effect } => {
                // The planted bug leaves abandoned tools running.
                if let Some(running) = self.effects.remove(&effect)
                    && !cfg!(piton_planted_shell_bug)
                {
                    running.task.abort();
                }
            }
        }
    }

    fn spawn_effect(&self, effect: Effect, life: u64, sender: mpsc::WeakSender<Event>) -> Running {
        match effect {
            Effect::CallModel { id, request } => {
                let model = Arc::clone(&self.ports.model);
                let task = tokio::spawn(async move {
                    let input = match model.call(id, request).await {
                        Ok(response) => Input::ModelResponded {
                            effect: id,
                            response,
                        },
                        Err(ModelError {
                            reason,
                            retryable,
                            retry_after,
                        }) => Input::ModelFailed {
                            effect: id,
                            reason,
                            retryable,
                            retry_after,
                        },
                    };
                    report(&sender, Event::Completed { life, input }).await;
                });
                Running { task, cancel: None }
            }
            Effect::RunTool { id, call } => {
                let tools = Arc::clone(&self.ports.tools);
                let (switch, signal) = CancelSignal::new();
                let task = tokio::spawn(async move {
                    let asked = signal.clone();
                    let input = match tools.run(id, call, signal).await {
                        ToolCompletion::Returned { output, is_error } => Input::ToolReturned {
                            effect: id,
                            output,
                            is_error,
                        },
                        ToolCompletion::Stopped if asked.is_cancelled() => {
                            Input::EffectCancelled { effect: id }
                        }
                        // Stopping unasked, as a host shutting down might, is
                        // the tool failing.
                        ToolCompletion::Stopped => Input::ToolReturned {
                            effect: id,
                            output: Json::string("the tool stopped without being asked to"),
                            is_error: true,
                        },
                    };
                    report(&sender, Event::Completed { life, input }).await;
                });
                Running {
                    task,
                    cancel: Some(switch),
                }
            }
        }
    }

    fn stop_everything(&mut self) {
        for running in std::mem::take(&mut self.effects).into_values() {
            running.task.abort();
        }
        if let Some(append) = self.append.take() {
            append.abort();
        }
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop_everything();
    }
}

/// How long an append may go unanswered before it counts as failed.
const APPEND_TIMEOUT: Duration = Duration::from_secs(30);

/// How many appends may fail in a row before the session gives up.
const MAX_FAILED_APPENDS: u32 = 8;

/// The wait before recovering after `failures` appends failed in a row:
/// doubling from 10ms, at most 2s.
fn recovery_backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(8);
    let millis = 10u64.checked_shl(doublings).unwrap_or(u64::MAX).min(2_000);
    Duration::from_millis(millis)
}

/// Hands `event` to the pump, if the session is still running.
async fn report(sender: &mpsc::WeakSender<Event>, event: Event) {
    if let Some(sender) = sender.upgrade() {
        sender.send(event).await.unwrap_or_default();
    }
}

/// Reads the whole log and rebuilds the session from it.
async fn recover(
    id: &SessionId,
    ports: &Ports,
) -> Result<(Session, Offset, JournalEntry), OpenError> {
    let records = ports
        .store
        .read(id, Offset::ZERO)
        .await
        .map_err(OpenError::Store)?;
    let now = ports.clock.now();
    let mut session = Session::from_log(now, &records).map_err(OpenError::Recovery)?;
    let log_len = end_of(Offset::ZERO, &records);
    let entry = JournalEntry::Recovered {
        now,
        log_len,
        submissions: drain(&mut session),
    };
    Ok((session, log_len, entry))
}

fn drain(session: &mut Session) -> Vec<Submission> {
    std::iter::from_fn(|| session.poll_submission()).collect()
}

/// The effect a completion input answers, if it answers one.
const fn completed_effect(input: &Input) -> Option<EffectId> {
    match input {
        Input::ModelResponded { effect, .. }
        | Input::ModelFailed { effect, .. }
        | Input::ToolReturned { effect, .. }
        | Input::EffectCancelled { effect } => Some(*effect),
        Input::Appended { .. } | Input::Submit { .. } | Input::CancelTurn { .. } | Input::Tick => {
            None
        }
    }
}

fn end_of(expected: Offset, records: &[Record]) -> Offset {
    let added = u64::try_from(records.len()).expect("an append's length fits in u64");
    Offset::new(
        expected
            .get()
            .checked_add(added)
            .expect("a log length fits in u64"),
    )
}

/// Completes at `deadline` by the runtime's clock; never, if there is none.
async fn sleep_until(ports: &Ports, deadline: Option<Timestamp>) {
    match deadline {
        Some(deadline) => {
            let wait = deadline.since(ports.clock.now());
            tokio::time::sleep(Duration::from_millis(wait.get())).await;
        }
        None => std::future::pending().await,
    }
}
