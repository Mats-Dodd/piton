//! The runtime's ports, simulated: each does what a real one might, faults
//! included, drawing every choice from the simulation's seed.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use piton_core::JournalEntry;
use piton_domain::{
    ContentBlock, EffectId, Json, Millis, ModelRequest, ModelResponse, Offset, Record, Role,
    SessionId, Timestamp, ToolCall, ToolName, ToolUseId,
};
use piton_runtime::{
    CancelSignal, Clock, LogStore, ModelError, ModelProvider, StoreError, TelemetryJournal,
    ToolCompletion, ToolHost,
};

use crate::world::{Happened, SimWorld};

/// Waits a random `low..high` milliseconds of simulated time, in steps of
/// five so that events often coincide and race.
async fn delay(world: &SimWorld, low: u64, high: u64) {
    let millis = world.pick(low, high);
    let coarse = millis.saturating_sub(millis.checked_rem(5).unwrap_or(0));
    tokio::time::sleep(Duration::from_millis(coarse)).await;
}

/// Storage that is slow, rejects appends, loses replies and applies appends
/// late, after the runtime has given up on them.
#[derive(Debug)]
pub(crate) struct Store(pub(crate) Arc<SimWorld>);

#[async_trait]
impl LogStore for Store {
    async fn read(&self, session: &SessionId, from: Offset) -> Result<Vec<Record>, StoreError> {
        delay(&self.0, 0, 3).await;
        if self.0.fault(20) {
            return Err(StoreError::new("unavailable".to_owned()));
        }
        self.0.store.read(session, from).await
    }

    async fn append(
        &self,
        session: &SessionId,
        expected: Offset,
        records: Vec<Record>,
    ) -> Result<(), StoreError> {
        if self.0.fault(100) {
            delay(&self.0, 20, 60).await;
        } else {
            delay(&self.0, 0, 3).await;
        }
        if self.0.fault(30) {
            return Err(StoreError::new("rejected".to_owned()));
        }
        if self.0.fault(30) {
            self.0.store.append_now(session, expected, records)?;
            return Err(StoreError::new("reply lost".to_owned()));
        }
        if self.0.fault(30) {
            let due =
                Timestamp::from_millis(SimClock::millis()).after(Millis::new(self.0.pick(1, 40)));
            self.0.delay_append(expected, records, due);
            delay(&self.0, 0, 10).await;
            return Err(StoreError::new("timed out".to_owned()));
        }
        self.0.store.append_now(session, expected, records)
    }
}

/// A model that is slow, hangs, fails, sometimes in ways worth retrying, and
/// sometimes replies with nonsense. When it does reply, it asks for tools
/// early in a conversation and finishes later.
#[derive(Debug)]
pub(crate) struct Model(pub(crate) Arc<SimWorld>);

#[async_trait]
impl ModelProvider for Model {
    async fn call(
        &self,
        effect: EffectId,
        request: ModelRequest,
    ) -> Result<ModelResponse, ModelError> {
        let reply = self.reply(effect, &request).await;
        self.0
            .happened(effect, Happened::ModelReplied(reply.clone()));
        reply
    }
}

impl Model {
    async fn reply(
        &self,
        effect: EffectId,
        request: &ModelRequest,
    ) -> Result<ModelResponse, ModelError> {
        let world = &self.0;
        delay(world, 1, 15).await;
        if world.fault(30) {
            std::future::pending::<()>().await;
        }
        if world.fault(50) {
            return Err(ModelError {
                reason: "overloaded".to_owned(),
                retryable: true,
                retry_after: world.chance(500).then(|| Millis::new(world.pick(0, 30))),
            });
        }
        if world.fault(15) {
            return Err(ModelError {
                reason: "rejected the request".to_owned(),
                retryable: false,
                retry_after: None,
            });
        }
        let replies = request
            .messages
            .iter()
            .filter(|message| message.role == Role::Assistant)
            .count();
        if replies >= 3 || !world.chance(600) {
            return Ok(ModelResponse {
                content: Vec::from([ContentBlock::Text {
                    text: "done".to_owned(),
                }]),
            });
        }
        let tools = ["search", "send_email", "missing"];
        let uses = world.pick(1, 3);
        let mut content: Vec<ContentBlock> = (0..uses)
            .map(|index| {
                let tool = usize::try_from(world.pick(0, 3)).unwrap_or(0);
                ContentBlock::ToolUse {
                    id: ToolUseId::new(format!("use-{}-{index}", effect.intent().get())),
                    name: ToolName::new(tools.get(tool).copied().unwrap_or("search").to_owned()),
                    input: Json::string("input"),
                }
            })
            .collect();
        if world.fault(15) {
            content.extend(content.clone());
        }
        Ok(ModelResponse { content })
    }
}

/// Tools that take a while, sometimes too long, sometimes fail, and
/// sometimes ignore being asked to stop. Records what really happened.
#[derive(Debug)]
pub(crate) struct Tools(pub(crate) Arc<SimWorld>);

#[async_trait]
impl ToolHost for Tools {
    async fn run(&self, effect: EffectId, call: ToolCall, cancel: CancelSignal) -> ToolCompletion {
        let world = &self.0;
        world.happened(effect, Happened::ToolStarted);
        let millis = if world.fault(200) {
            200
        } else {
            world.pick(1, 20)
        };
        let honours_cancel = !world.fault(400);
        let is_error = world.fault(100);
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(millis)) => {
                let output = Json::string(&format!("{} done", call.name.as_str()));
                world.happened(effect, Happened::ToolReturned { output: output.clone(), is_error });
                ToolCompletion::Returned { output, is_error }
            }
            () = cancel.cancelled(), if honours_cancel => {
                world.happened(effect, Happened::ToolStopped);
                ToolCompletion::Stopped
            }
        }
    }
}

/// Simulated time, continuous across restarts of the host.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SimClock;

impl SimClock {
    /// Milliseconds since the simulation started.
    pub(crate) fn millis() -> u64 {
        let elapsed = turmoil::sim_elapsed().unwrap_or_default();
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

/// The runtime's clock: simulated time, but jumping ahead and back as the
/// simulation skews it, as wall clocks do. A jump ahead makes results
/// arrive after deadlines the runtime has not yet noticed.
#[derive(Debug)]
pub(crate) struct SkewedClock(pub(crate) Arc<SimWorld>);

impl Clock for SkewedClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(SimClock::millis().saturating_add(self.0.skew()))
    }
}

/// Keeps the journal in the world, where it survives crashes.
#[derive(Debug)]
pub(crate) struct Journal(pub(crate) Arc<SimWorld>);

impl TelemetryJournal for Journal {
    fn record(&self, _session: &SessionId, entry: &JournalEntry) {
        self.0.record(entry);
    }
}
