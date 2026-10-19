use std::collections::{BTreeMap, BTreeSet};

use piton_core::{Effect, Input, Session, Submission};
use piton_domain::{
    ContentBlock, EffectId, Json, ModelResponse, Offset, Record, RecordBody, ReplayPolicy,
    RequestId, SCHEMA_VERSION, Timestamp, ToolName, ToolSpec, ToolUseId, TurnId,
};
use serde::{Deserialize, Serialize};

use crate::Violation;

/// The tools a session has and how much nondeterminism the environment may
/// inject. Not part of the explored state.
#[derive(Clone, Debug)]
pub struct WorldConfig {
    pub tools: Vec<ToolSpec>,
    /// Each entry is a reply the model may give: one tool use per name.
    /// Names outside `tools` exercise unknown-tool handling.
    pub tool_replies: Vec<Vec<ToolName>>,
    /// Also offer a reply that reuses a tool use id.
    pub invalid_replies: bool,
    /// How many replies may ask for tools, across the whole run.
    pub tool_rounds: u8,
    pub submits: u8,
    pub cancels: u8,
    pub crashes: u8,
    pub advance_time: bool,
}

impl WorldConfig {
    /// Small enough for Stateright to explore every state, large enough to
    /// reach parallel tools, an unknown tool, an invalid reply, a cancel and a
    /// crash in one run.
    #[must_use]
    pub fn exhaustive() -> Self {
        Self {
            tools: tools(),
            tool_replies: Vec::from([
                Vec::from([search(), send_email()]),
                Vec::from([ToolName::new("missing".to_owned())]),
            ]),
            invalid_replies: true,
            tool_rounds: 2,
            submits: 2,
            cancels: 1,
            crashes: 2,
            advance_time: false,
        }
    }

    /// Larger budgets for random walks, which go deep rather than wide.
    #[must_use]
    pub fn deep() -> Self {
        Self {
            tools: tools(),
            tool_replies: Vec::from([
                Vec::from([search()]),
                Vec::from([send_email()]),
                Vec::from([search(), send_email()]),
                Vec::from([ToolName::new("missing".to_owned()), search()]),
            ]),
            invalid_replies: true,
            tool_rounds: 6,
            submits: 4,
            cancels: 2,
            crashes: 3,
            advance_time: true,
        }
    }
}

/// A tool that is safe to run again.
fn search() -> ToolName {
    ToolName::new("search".to_owned())
}

/// A tool that must never run twice.
fn send_email() -> ToolName {
    ToolName::new("send_email".to_owned())
}

fn tools() -> Vec<ToolSpec> {
    Vec::from([
        ToolSpec {
            name: search(),
            replay: ReplayPolicy::Redispatch,
        },
        ToolSpec {
            name: send_email(),
            replay: ReplayPolicy::Interrupt,
        },
    ])
}

/// Something the environment does. Each maps to one session input, except
/// crashes and the passage of time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EnvAction {
    /// Storage durably accepts the outstanding append.
    AckAppend,
    Respond {
        effect: EffectId,
        response: ModelResponse,
    },
    FailModel {
        effect: EffectId,
    },
    ReturnTool {
        effect: EffectId,
        output: Json,
        is_error: bool,
    },
    /// A dispatched effect honours a cancellation request.
    ConfirmCancel {
        effect: EffectId,
    },
    Submit {
        request_id: RequestId,
        text: String,
    },
    CancelTurn {
        turn: TurnId,
    },
    /// The process dies and restarts from its durable log. The outstanding
    /// append may or may not have reached storage.
    Crash {
        append_lands: bool,
    },
    Advance {
        millis: u64,
    },
}

/// Everything outside the session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Env {
    now: Timestamp,
    durable: Vec<Record>,
    in_flight: Option<Vec<Record>>,
    dispatched: BTreeMap<EffectId, Effect>,
    cancel_requested: BTreeSet<EffectId>,
    /// Tool calls with `ReplayPolicy::Interrupt` that were ever dispatched.
    interrupt_dispatches: BTreeSet<EffectId>,
    submits: u8,
    cancels: u8,
    crashes: u8,
    tool_rounds: u8,
    /// The first rule the session broke while talking to the environment.
    violation: Option<Violation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct World {
    session: Session,
    env: Env,
}

impl World {
    /// A session whose log holds only its creation record.
    ///
    /// # Panics
    ///
    /// Panics if the core rejects a freshly created log.
    #[must_use]
    pub fn new(config: &WorldConfig) -> Self {
        let created = Record {
            offset: Offset::ZERO,
            at: Timestamp::EPOCH,
            body: RecordBody::SessionCreated {
                schema: SCHEMA_VERSION,
                tools: config.tools.clone(),
            },
        };
        let durable = Vec::from([created]);
        let mut session =
            Session::from_log(Timestamp::EPOCH, &durable).expect("a fresh log recovers");
        let mut env = Env {
            now: Timestamp::EPOCH,
            durable,
            in_flight: None,
            dispatched: BTreeMap::new(),
            cancel_requested: BTreeSet::new(),
            interrupt_dispatches: BTreeSet::new(),
            submits: 0,
            cancels: 0,
            crashes: 0,
            tool_rounds: 0,
            violation: None,
        };
        env.drain(&mut session);
        Self { session, env }
    }

    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    #[must_use]
    pub const fn env(&self) -> &Env {
        &self.env
    }
}

impl Env {
    #[must_use]
    pub fn durable(&self) -> &[Record] {
        &self.durable
    }

    #[must_use]
    pub fn in_flight(&self) -> &[Record] {
        self.in_flight.as_deref().unwrap_or_default()
    }

    #[must_use]
    pub const fn violation(&self) -> Option<&Violation> {
        self.violation.as_ref()
    }

    /// Collects the session's submissions, enforcing the rules the shell
    /// relies on as it goes.
    fn drain(&mut self, session: &mut Session) -> Vec<Submission> {
        let mut submissions = Vec::new();
        while let Some(submission) = session.poll_submission() {
            if let Err(violation) = self.accept(&submission) {
                self.violation.get_or_insert(violation);
            }
            submissions.push(submission);
        }
        submissions
    }

    fn accept(&mut self, submission: &Submission) -> Result<(), Violation> {
        match submission {
            Submission::Append { expected, records } => {
                if self.in_flight.is_some() {
                    return Err(Violation::ConcurrentAppend);
                }
                let tail = log_len(&self.durable);
                if *expected != tail {
                    return Err(Violation::AppendNotAtTail {
                        expected: *expected,
                        tail,
                    });
                }
                self.in_flight = Some(records.clone());
            }
            Submission::Dispatch { effect } => {
                let id = effect.id();
                let intent = usize::try_from(id.intent().get())
                    .ok()
                    .and_then(|index| self.durable.get(index))
                    .ok_or(Violation::DispatchBeforeDurable(id))?;
                let replay = match (&intent.body, effect) {
                    (
                        RecordBody::ModelIntent { request, .. },
                        Effect::CallModel {
                            request: dispatched,
                            ..
                        },
                    ) if request == dispatched => ReplayPolicy::Redispatch,
                    (
                        RecordBody::ToolIntent { call, replay, .. },
                        Effect::RunTool {
                            call: dispatched, ..
                        },
                    ) if call == dispatched => *replay,
                    _ => return Err(Violation::DispatchMismatch(id)),
                };
                if self.dispatched.insert(id, effect.clone()).is_some() {
                    return Err(Violation::DoubleDispatch(id));
                }
                if replay == ReplayPolicy::Interrupt && !self.interrupt_dispatches.insert(id) {
                    return Err(Violation::UnsafeRedispatch(id));
                }
            }
            Submission::Cancel { effect } => {
                if !self.dispatched.contains_key(effect) {
                    return Err(Violation::CancelUndispatched(*effect));
                }
                self.cancel_requested.insert(*effect);
            }
        }
        Ok(())
    }

    /// Removes a dispatched effect the environment is about to complete.
    fn complete(&mut self, effect: EffectId) {
        self.dispatched.remove(&effect);
        self.cancel_requested.remove(&effect);
    }
}

/// What the environment may do next, in a deterministic order.
#[must_use]
pub fn enabled(config: &WorldConfig, world: &World) -> Vec<EnvAction> {
    let env = &world.env;
    let mut actions = Vec::new();
    if env.in_flight.is_some() {
        actions.push(EnvAction::AckAppend);
    }
    for (id, effect) in &env.dispatched {
        match effect {
            Effect::CallModel { .. } => {
                actions.push(EnvAction::Respond {
                    effect: *id,
                    response: ModelResponse {
                        content: Vec::from([ContentBlock::Text {
                            text: "done".to_owned(),
                        }]),
                    },
                });
                actions.push(EnvAction::FailModel { effect: *id });
                if env.tool_rounds < config.tool_rounds {
                    for names in &config.tool_replies {
                        actions.push(EnvAction::Respond {
                            effect: *id,
                            response: tool_reply(*id, names),
                        });
                    }
                    if config.invalid_replies
                        && let Some(names) = config.tool_replies.first()
                    {
                        let mut reply = tool_reply(*id, names);
                        reply.content.extend(reply.content.clone());
                        actions.push(EnvAction::Respond {
                            effect: *id,
                            response: reply,
                        });
                    }
                }
            }
            Effect::RunTool { .. } => actions.push(EnvAction::ReturnTool {
                effect: *id,
                output: Json::string("ok"),
                is_error: false,
            }),
        }
        if env.cancel_requested.contains(id) {
            actions.push(EnvAction::ConfirmCancel { effect: *id });
        }
    }
    if env.submits < config.submits {
        actions.push(EnvAction::Submit {
            request_id: request_id(env.submits),
            text: "hello".to_owned(),
        });
        if env.submits > 0 {
            actions.push(EnvAction::Submit {
                request_id: request_id(0),
                text: "hello".to_owned(),
            });
        }
    }
    if env.cancels < config.cancels
        && let Some(turn) = world.session.active_turn()
    {
        actions.push(EnvAction::CancelTurn { turn });
    }
    if env.crashes < config.crashes {
        if env.in_flight.is_some() {
            actions.push(EnvAction::Crash { append_lands: true });
        }
        actions.push(EnvAction::Crash {
            append_lands: false,
        });
    }
    if config.advance_time {
        actions.push(EnvAction::Advance { millis: 1 });
    }
    actions
}

/// Does `action` to the world and returns what the session submitted in
/// response.
pub fn step(world: &mut World, action: EnvAction) -> Vec<Submission> {
    let env = &mut world.env;
    let input = match action {
        EnvAction::AckAppend => {
            if let Some(records) = env.in_flight.take() {
                env.durable.extend(records);
            }
            Input::Appended {
                end: log_len(&env.durable),
            }
        }
        EnvAction::Respond { effect, response } => {
            env.complete(effect);
            if !response.tool_calls().is_empty() {
                env.tool_rounds = env.tool_rounds.saturating_add(1);
            }
            Input::ModelResponded { effect, response }
        }
        EnvAction::FailModel { effect } => {
            env.complete(effect);
            Input::ModelFailed {
                effect,
                reason: "provider error".to_owned(),
            }
        }
        EnvAction::ReturnTool {
            effect,
            output,
            is_error,
        } => {
            env.complete(effect);
            Input::ToolReturned {
                effect,
                output,
                is_error,
            }
        }
        EnvAction::ConfirmCancel { effect } => {
            env.complete(effect);
            Input::EffectCancelled { effect }
        }
        EnvAction::Submit { request_id, text } => {
            env.submits = env.submits.saturating_add(1);
            Input::Submit { request_id, text }
        }
        EnvAction::CancelTurn { turn } => {
            env.cancels = env.cancels.saturating_add(1);
            Input::CancelTurn { turn }
        }
        EnvAction::Crash { append_lands } => {
            env.crashes = env.crashes.saturating_add(1);
            if let Some(records) = env.in_flight.take()
                && append_lands
            {
                env.durable.extend(records);
            }
            env.dispatched.clear();
            env.cancel_requested.clear();
            match Session::from_log(env.now, &env.durable) {
                Ok(session) => world.session = session,
                Err(error) => {
                    env.violation
                        .get_or_insert(Violation::Unrecoverable(error.to_string()));
                }
            }
            return env.drain(&mut world.session);
        }
        EnvAction::Advance { millis } => {
            env.now = Timestamp::from_millis(env.now.millis().saturating_add(millis));
            return Vec::new();
        }
    };
    world.session.handle(env.now, input);
    env.drain(&mut world.session)
}

fn log_len(records: &[Record]) -> Offset {
    Offset::new(u64::try_from(records.len()).expect("a log length fits in u64"))
}

fn request_id(index: u8) -> RequestId {
    RequestId::new(format!("request-{index}"))
}

fn tool_reply(effect: EffectId, names: &[ToolName]) -> ModelResponse {
    let content = names
        .iter()
        .enumerate()
        .map(|(index, name)| ContentBlock::ToolUse {
            id: ToolUseId::new(format!("use-{}-{index}", effect.intent().get())),
            name: name.clone(),
            input: Json::string("input"),
        })
        .collect();
    ModelResponse { content }
}
