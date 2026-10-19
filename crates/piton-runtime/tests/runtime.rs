//! The runtime end to end on paused tokio time, with every session's journal
//! replayed through the model harness afterwards.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use piton_core::JournalEntry;
use piton_domain::{
    ContentBlock, EffectId, Json, Millis, ModelOutcome, ModelPolicy, ModelRequest, ModelResponse,
    Offset, Record, RecordBody, RequestId, RetryPolicy, Safety, SessionId, StopReason, Timestamp,
    ToolCall, ToolName, ToolOutcome, ToolSpec, ToolUseId, TurnEnd, TurnId, UnknownCause,
};
use piton_model::{WorldConfig, validate};
use piton_runtime::{
    CancelSignal, LogStore, MemLogStore, ModelError, ModelProvider, Ports, Runtime, SessionHandle,
    StoreError, TelemetryJournal, TokioClock, ToolCompletion, ToolHost,
};

fn tool_specs() -> Vec<ToolSpec> {
    Vec::from([
        ToolSpec {
            name: ToolName::new("search".to_owned()),
            safety: Safety::Idempotent,
            timeout: Millis::new(60_000),
            cancel_grace: Millis::ZERO,
        },
        ToolSpec {
            name: ToolName::new("send_email".to_owned()),
            safety: Safety::Unsafe,
            timeout: Millis::new(60_000),
            cancel_grace: Millis::new(5_000),
        },
    ])
}

const MODEL_POLICY: ModelPolicy = ModelPolicy {
    timeout: Millis::new(60_000),
    retry: RetryPolicy {
        max_attempts: 3,
        initial_backoff: Millis::new(100),
        max_backoff: Millis::new(1_000),
    },
};

/// Replies in order, then says "done" forever.
#[derive(Debug, Default)]
struct ScriptedModel {
    replies: Mutex<VecDeque<Result<ModelResponse, ModelError>>>,
}

impl ScriptedModel {
    fn new(replies: impl IntoIterator<Item = Result<ModelResponse, ModelError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ModelProvider for ScriptedModel {
    async fn call(
        &self,
        _effect: EffectId,
        _request: ModelRequest,
    ) -> Result<ModelResponse, ModelError> {
        let next = self.replies.lock().expect("not poisoned").pop_front();
        next.unwrap_or_else(|| Ok(text("done")))
    }
}

/// `search` returns at once. `send_email` waits to be cancelled, then
/// stops, unless it is stubborn, in which case it never returns.
#[derive(Debug, Default)]
struct Tools {
    stubborn: bool,
}

#[async_trait]
impl ToolHost for Tools {
    async fn run(&self, _effect: EffectId, call: ToolCall, cancel: CancelSignal) -> ToolCompletion {
        if call.name.as_str() == "search" {
            return ToolCompletion::Returned {
                output: Json::string("found it"),
                is_error: false,
            };
        }
        if self.stubborn {
            std::future::pending::<()>().await;
        }
        cancel.cancelled().await;
        ToolCompletion::Stopped
    }
}

/// Lands the `failing`-th append (counting from one), then reports failure,
/// as if the reply were lost.
#[derive(Debug)]
struct LosesReply {
    inner: MemLogStore,
    appends: AtomicUsize,
    failing: usize,
}

#[async_trait]
impl LogStore for LosesReply {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        self.inner.read(session, from).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        self.inner.append(session, expected, records).await?;
        if self
            .appends
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1)
            == self.failing
        {
            return Err(StoreError::new("reply lost".to_owned()));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Journal(Mutex<Vec<JournalEntry>>);

impl TelemetryJournal for Journal {
    fn record(&self, _session: &SessionId, entry: &JournalEntry) {
        self.0.lock().expect("not poisoned").push(entry.clone());
    }
}

struct Harness {
    session: SessionHandle,
    journal: Arc<Journal>,
}

impl Harness {
    async fn start(store: Arc<dyn LogStore>, model: ScriptedModel, tools: Tools) -> Self {
        let journal = Arc::new(Journal::default());
        let journal_port: Arc<dyn TelemetryJournal> = Arc::<Journal>::clone(&journal);
        let runtime = Runtime::new(Ports {
            store,
            model: Arc::new(model),
            tools: Arc::new(tools),
            clock: Arc::new(TokioClock::starting_at(Timestamp::EPOCH)),
            journal: journal_port,
        });
        let session = runtime
            .create(
                SessionId::new("session".to_owned()),
                tool_specs(),
                MODEL_POLICY,
            )
            .await
            .expect("the session is created");
        Self { session, journal }
    }

    async fn submit(&self, text: &str) {
        self.session
            .submit(RequestId::new("request".to_owned()), text.to_owned())
            .await
            .expect("the session accepts input");
    }

    /// Durable records up to and including the first that matches.
    async fn records_until(&self, done: impl Fn(&RecordBody) -> bool) -> Vec<Record> {
        let mut follower = self.session.follow(Offset::ZERO);
        let mut records = Vec::new();
        loop {
            let record = follower.next().await.expect("the log is readable");
            let finished = done(&record.body);
            records.push(record);
            if finished {
                return records;
            }
        }
    }

    /// The journal so far replays through the model harness.
    fn assert_journal_replays(&self) {
        let entries = self.journal.0.lock().expect("not poisoned").clone();
        let replayed = validate(
            &WorldConfig::replaying(tool_specs(), MODEL_POLICY),
            &entries,
        );
        assert!(
            replayed.is_ok(),
            "the journal diverges from the model: {:?}\n{entries:#?}",
            replayed.err()
        );
    }
}

fn text(text: &str) -> ModelResponse {
    ModelResponse {
        content: Vec::from([ContentBlock::Text {
            text: text.to_owned(),
        }]),
    }
}

fn use_tool(name: &str) -> ModelResponse {
    ModelResponse {
        content: Vec::from([ContentBlock::ToolUse {
            id: ToolUseId::new(format!("use-{name}")),
            name: ToolName::new(name.to_owned()),
            input: Json::string("input"),
        }]),
    }
}

const fn turn_ended(body: &RecordBody) -> bool {
    matches!(body, RecordBody::TurnEnded { .. })
}

fn ended_with(records: &[Record]) -> Option<TurnEnd> {
    records.iter().find_map(|record| {
        if let RecordBody::TurnEnded { reason, .. } = &record.body {
            Some(*reason)
        } else {
            None
        }
    })
}

fn tool_outcomes(records: &[Record]) -> Vec<ToolOutcome> {
    records
        .iter()
        .filter_map(|record| {
            if let RecordBody::ToolResolved { outcome, .. } = &record.body {
                Some(outcome.clone())
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_turn_runs_a_tool_and_completes() {
    let harness = Harness::start(
        Arc::new(MemLogStore::new()),
        ScriptedModel::new([Ok(use_tool("search"))]),
        Tools::default(),
    )
    .await;
    harness.submit("find piton").await;
    let records = harness.records_until(turn_ended).await;
    assert_eq!(
        ended_with(&records),
        Some(TurnEnd::Completed),
        "{records:#?}"
    );
    assert_eq!(
        tool_outcomes(&records),
        [ToolOutcome::Returned {
            output: Json::string("found it"),
            is_error: false,
        }],
        "the tool's output is recorded"
    );
    harness.assert_journal_replays();
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_turn_stops_an_unsafe_tool() {
    let harness = Harness::start(
        Arc::new(MemLogStore::new()),
        ScriptedModel::new([Ok(use_tool("send_email"))]),
        Tools::default(),
    )
    .await;
    harness.submit("email them").await;
    let intent = harness
        .records_until(|body| matches!(body, RecordBody::ToolIntent { .. }))
        .await;
    let turn = intent
        .iter()
        .find(|record| matches!(record.body, RecordBody::TurnStarted { .. }))
        .map(|record| TurnId::new(record.offset))
        .expect("a turn started");
    // Let the tool start before cancelling it.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    harness
        .session
        .cancel(turn)
        .await
        .expect("the session is running");
    let records = harness.records_until(turn_ended).await;
    assert_eq!(
        ended_with(&records),
        Some(TurnEnd::Cancelled),
        "{records:#?}"
    );
    assert_eq!(
        tool_outcomes(&records),
        [ToolOutcome::Stopped {
            reason: StopReason::TurnCancelled
        }],
        "the tool confirmed it stopped"
    );
    harness.assert_journal_replays();
}

#[tokio::test(start_paused = true)]
async fn an_unsafe_tool_that_ignores_a_cancel_is_abandoned_after_its_grace() {
    let harness = Harness::start(
        Arc::new(MemLogStore::new()),
        ScriptedModel::new([Ok(use_tool("send_email"))]),
        Tools { stubborn: true },
    )
    .await;
    harness.submit("email them").await;
    harness
        .records_until(|body| matches!(body, RecordBody::ToolIntent { .. }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    harness
        .session
        .cancel(TurnId::new(Offset::new(3)))
        .await
        .expect("the session is running");
    let records = harness.records_until(turn_ended).await;
    assert_eq!(
        tool_outcomes(&records),
        [ToolOutcome::Unknown {
            cause: UnknownCause::Abandoned
        }],
        "the tool may have run, and the log says so"
    );
    harness.assert_journal_replays();
}

#[tokio::test(start_paused = true)]
async fn a_retryable_model_failure_is_retried() {
    let harness = Harness::start(
        Arc::new(MemLogStore::new()),
        ScriptedModel::new([Err(ModelError {
            reason: "overloaded".to_owned(),
            retryable: true,
            retry_after: Some(Millis::new(500)),
        })]),
        Tools::default(),
    )
    .await;
    harness.submit("hello").await;
    let records = harness.records_until(turn_ended).await;
    assert_eq!(
        ended_with(&records),
        Some(TurnEnd::Completed),
        "{records:#?}"
    );
    let attempts: Vec<Timestamp> = records
        .iter()
        .filter(|record| matches!(record.body, RecordBody::ModelIntent { .. }))
        .map(|record| record.at)
        .collect();
    assert_eq!(attempts.len(), 2, "one failure, one retry");
    let waited = attempts
        .last()
        .zip(attempts.first())
        .map(|(retry, first)| retry.since(*first));
    assert!(
        waited >= Some(Millis::new(500)),
        "the retry waited out retry_after: {waited:?}"
    );
    assert!(
        records.iter().any(|record| matches!(
            &record.body,
            RecordBody::ModelResolved {
                outcome: ModelOutcome::Failed { .. },
                ..
            }
        )),
        "the failure is recorded"
    );
    harness.assert_journal_replays();
}

#[tokio::test(start_paused = true)]
async fn a_lost_append_reply_is_survived_by_recovering_from_the_log() {
    let store = Arc::new(LosesReply {
        inner: MemLogStore::new(),
        appends: AtomicUsize::new(0),
        // Creation, the first life's fence, then the submission.
        failing: 3,
    });
    let harness = Harness::start(
        store,
        ScriptedModel::new([Ok(use_tool("search"))]),
        Tools::default(),
    )
    .await;
    harness.submit("find piton").await;
    let records = harness.records_until(turn_ended).await;
    assert_eq!(
        ended_with(&records),
        Some(TurnEnd::Completed),
        "{records:#?}"
    );
    let lives = records
        .iter()
        .filter(|record| record.body == RecordBody::SessionResumed)
        .count();
    assert_eq!(lives, 2, "the session recovered once: {records:#?}");
    harness.assert_journal_replays();
}
