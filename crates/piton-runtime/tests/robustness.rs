//! The runtime against ports that misbehave in ways the contract allows,
//! on paused tokio time. Every session's journal must still replay through
//! the model harness.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use piton_core::JournalEntry;
use piton_domain::{
    ContentBlock, EffectId, InvalidPolicy, Json, Millis, ModelPolicy, ModelRequest, ModelResponse,
    Offset, Record, RecordBody, RequestId, RetryPolicy, Safety, SessionId, Timestamp, ToolCall,
    ToolName, ToolOutcome, ToolSpec, ToolUseId, TurnEnd,
};
use piton_model::{WorldConfig, validate};
use piton_runtime::{
    CancelSignal, FollowError, LogStore, MemLogStore, ModelError, ModelProvider, OpenError, Ports,
    Runtime, SessionHandle, StoreError, TelemetryJournal, TokioClock, ToolCompletion, ToolHost,
};

fn tool_specs() -> Vec<ToolSpec> {
    Vec::from([ToolSpec {
        name: ToolName::new("search".to_owned()),
        safety: Safety::Idempotent,
        timeout: Millis::new(50),
        cancel_grace: Millis::ZERO,
    }])
}

const MODEL_POLICY: ModelPolicy = ModelPolicy {
    timeout: Millis::new(60_000),
    retry: RetryPolicy {
        max_attempts: 3,
        initial_backoff: Millis::new(100),
        max_backoff: Millis::new(1_000),
    },
};

/// Asks for `search` once, then says "done".
#[derive(Debug, Default)]
struct UsesSearch(AtomicUsize);

#[async_trait]
impl ModelProvider for UsesSearch {
    async fn call(
        &self,
        _effect: EffectId,
        _request: ModelRequest,
    ) -> Result<ModelResponse, ModelError> {
        let content = if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            ContentBlock::ToolUse {
                id: ToolUseId::new("use-search".to_owned()),
                name: ToolName::new("search".to_owned()),
                input: Json::string("input"),
            }
        } else {
            ContentBlock::Text {
                text: "done".to_owned(),
            }
        };
        Ok(ModelResponse {
            content: Vec::from([content]),
        })
    }
}

/// Returns at once.
#[derive(Debug)]
struct Quick;

#[async_trait]
impl ToolHost for Quick {
    async fn run(
        &self,
        _effect: EffectId,
        _call: ToolCall,
        _cancel: CancelSignal,
    ) -> ToolCompletion {
        ToolCompletion::Returned {
            output: Json::string("found it"),
            is_error: false,
        }
    }
}

#[derive(Debug, Default)]
struct Journal(Mutex<Vec<JournalEntry>>);

impl TelemetryJournal for Journal {
    fn record(&self, _session: &SessionId, entry: &JournalEntry) {
        self.0.lock().expect("not poisoned").push(entry.clone());
    }
}

impl Journal {
    fn entries(&self) -> Vec<JournalEntry> {
        self.0.lock().expect("not poisoned").clone()
    }

    fn recoveries(&self) -> usize {
        self.entries()
            .iter()
            .filter(|entry| matches!(entry, JournalEntry::Recovered { .. }))
            .count()
    }

    fn assert_replays(&self) {
        let entries = self.entries();
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

fn runtime(store: Arc<dyn LogStore>, tools: Arc<dyn ToolHost>, journal: &Arc<Journal>) -> Runtime {
    let journal: Arc<dyn TelemetryJournal> = Arc::<Journal>::clone(journal);
    Runtime::new(Ports {
        store,
        model: Arc::new(UsesSearch::default()),
        tools,
        clock: Arc::new(TokioClock::starting_at(Timestamp::EPOCH)),
        journal,
    })
}

fn session_id() -> SessionId {
    SessionId::new("session".to_owned())
}

async fn submit(session: &SessionHandle) {
    session
        .submit(RequestId::new("request".to_owned()), "go".to_owned())
        .await
        .expect("the session accepts input");
}

/// Durable records until the first turn ends.
async fn first_turn(session: &SessionHandle) -> Vec<Record> {
    let mut follower = session.follow(Offset::ZERO);
    let mut records = Vec::new();
    loop {
        let record = follower.next().await.expect("the log is readable");
        let ended = matches!(record.body, RecordBody::TurnEnded { .. });
        records.push(record);
        if ended {
            return records;
        }
    }
}

/// Lands every append after the session's creation, then reports failure.
#[derive(Debug, Default)]
struct AlwaysLosesReplies(MemLogStore);

#[async_trait]
impl LogStore for AlwaysLosesReplies {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        self.0.read(session, from).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        self.0.append(session, expected, records).await?;
        if expected == Offset::ZERO {
            Ok(())
        } else {
            Err(StoreError::new("reply lost".to_owned()))
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_store_that_keeps_failing_closes_the_session_after_backing_off() {
    let journal = Arc::new(Journal::default());
    let started = tokio::time::Instant::now();
    let session = runtime(
        Arc::new(AlwaysLosesReplies::default()),
        Arc::new(Quick),
        &journal,
    )
    .create(session_id(), tool_specs(), MODEL_POLICY)
    .await
    .expect("the session is created");
    let mut follower = session.follow(Offset::new(u64::MAX));
    let closed = tokio::time::timeout(Duration::from_secs(3_600), follower.next()).await;
    assert!(
        matches!(closed, Ok(Err(FollowError::Closed))),
        "the session gave up: {closed:?}"
    );
    assert!(
        journal.recoveries() <= 12,
        "recovered {} times before giving up",
        journal.recoveries()
    );
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "recoveries backed off: only {:?} passed",
        started.elapsed()
    );
    journal.assert_replays();
}

#[tokio::test(start_paused = true)]
async fn dropping_every_handle_stops_the_session() {
    let journal = Arc::new(Journal::default());
    let session = runtime(Arc::new(MemLogStore::new()), Arc::new(Quick), &journal)
        .create(session_id(), tool_specs(), MODEL_POLICY)
        .await
        .expect("the session is created");
    let mut follower = session.follow(Offset::new(u64::MAX));
    drop(session);
    let closed = tokio::time::timeout(Duration::from_secs(3_600), follower.next()).await;
    assert!(
        matches!(closed, Ok(Err(FollowError::Closed))),
        "the session stopped: {closed:?}"
    );
}

/// Answers that it stopped without being asked, as a host shutting down
/// might.
#[derive(Debug)]
struct StopsUnasked;

#[async_trait]
impl ToolHost for StopsUnasked {
    async fn run(
        &self,
        _effect: EffectId,
        _call: ToolCall,
        _cancel: CancelSignal,
    ) -> ToolCompletion {
        ToolCompletion::Stopped
    }
}

#[tokio::test(start_paused = true)]
async fn a_tool_that_stops_unasked_fails_instead() {
    let journal = Arc::new(Journal::default());
    let session = runtime(
        Arc::new(MemLogStore::new()),
        Arc::new(StopsUnasked),
        &journal,
    )
    .create(session_id(), tool_specs(), MODEL_POLICY)
    .await
    .expect("the session is created");
    submit(&session).await;
    let records = first_turn(&session).await;
    let outcome = records.iter().find_map(|record| {
        if let RecordBody::ToolResolved { outcome, .. } = &record.body {
            Some(outcome.clone())
        } else {
            None
        }
    });
    assert!(
        matches!(outcome, Some(ToolOutcome::Returned { is_error: true, .. })),
        "an unasked stop is the tool failing: {outcome:?}"
    );
    journal.assert_replays();
}

/// Never returns, but hands its cancel signal to work that outlives it.
#[derive(Debug, Default)]
struct Detached {
    told_to_stop: Arc<AtomicBool>,
}

#[async_trait]
impl ToolHost for Detached {
    async fn run(
        &self,
        _effect: EffectId,
        _call: ToolCall,
        cancel: CancelSignal,
    ) -> ToolCompletion {
        let told_to_stop = Arc::clone(&self.told_to_stop);
        tokio::spawn(async move {
            cancel.cancelled().await;
            told_to_stop.store(true, Ordering::SeqCst);
        });
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_tool_is_told_to_stop() {
    let journal = Arc::new(Journal::default());
    let tools = Arc::new(Detached::default());
    let told_to_stop = Arc::clone(&tools.told_to_stop);
    let session = runtime(Arc::new(MemLogStore::new()), tools, &journal)
        .create(session_id(), tool_specs(), MODEL_POLICY)
        .await
        .expect("the session is created");
    submit(&session).await;
    let records = first_turn(&session).await;
    assert!(
        records.iter().any(|record| matches!(
            record.body,
            RecordBody::TurnEnded {
                reason: TurnEnd::Completed,
                ..
            }
        )),
        "the turn finished without the tool"
    );
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert!(
        told_to_stop.load(Ordering::SeqCst),
        "the tool's detached work was told to stop"
    );
    journal.assert_replays();
}

/// Lands the session's creation, then reports failure.
#[derive(Debug, Default)]
struct LosesCreationReply(MemLogStore);

#[async_trait]
impl LogStore for LosesCreationReply {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        self.0.read(session, from).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        self.0.append(session, expected, records).await?;
        if expected == Offset::ZERO {
            Err(StoreError::new("reply lost".to_owned()))
        } else {
            Ok(())
        }
    }
}

#[tokio::test(start_paused = true)]
async fn creating_a_session_survives_a_lost_reply() {
    let journal = Arc::new(Journal::default());
    let runtime = runtime(
        Arc::new(LosesCreationReply::default()),
        Arc::new(Quick),
        &journal,
    );
    let created = runtime
        .create(session_id(), tool_specs(), MODEL_POLICY)
        .await;
    assert!(created.is_ok(), "the session exists: {:?}", created.err());
    let again = runtime
        .create(session_id(), Vec::new(), MODEL_POLICY)
        .await
        .err();
    assert_eq!(
        again,
        Some(OpenError::AlreadyExists),
        "a different session by the same name is refused"
    );
}

/// Never answers the first append after the session's first life.
#[derive(Debug, Default)]
struct HangsOnce {
    inner: MemLogStore,
    appends: AtomicUsize,
}

#[async_trait]
impl LogStore for HangsOnce {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        self.inner.read(session, from).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        // Creation, then the first life's fence, then the one that hangs.
        if self.appends.fetch_add(1, Ordering::SeqCst) == 2 {
            std::future::pending::<()>().await;
        }
        self.inner.append(session, expected, records).await
    }
}

#[tokio::test(start_paused = true)]
async fn an_append_that_never_answers_is_given_up_on() {
    let journal = Arc::new(Journal::default());
    let session = runtime(Arc::new(HangsOnce::default()), Arc::new(Quick), &journal)
        .create(session_id(), tool_specs(), MODEL_POLICY)
        .await
        .expect("the session is created");
    submit(&session).await;
    // The hung append held the submission, so submit again, as clients do.
    tokio::time::sleep(Duration::from_secs(60)).await;
    submit(&session).await;
    let records = tokio::time::timeout(Duration::from_secs(3_600), first_turn(&session)).await;
    assert!(
        records.is_ok(),
        "the session recovered from the hung append"
    );
    assert!(journal.recoveries() >= 2, "it recovered");
    journal.assert_replays();
}

#[tokio::test(start_paused = true)]
async fn a_session_with_an_unsound_policy_is_never_created() {
    let journal = Arc::new(Journal::default());
    let store = Arc::new(MemLogStore::new());
    let unsound = ModelPolicy {
        timeout: Millis::ZERO,
        ..MODEL_POLICY
    };
    let created = runtime(Arc::<MemLogStore>::clone(&store), Arc::new(Quick), &journal)
        .create(session_id(), tool_specs(), unsound)
        .await
        .err();
    assert_eq!(
        created,
        Some(OpenError::InvalidPolicy(InvalidPolicy::ZeroModelTimeout)),
        "the policy is refused"
    );
    assert!(
        store.records(&session_id()).is_empty(),
        "nothing was written"
    );
}
