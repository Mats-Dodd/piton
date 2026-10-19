use std::collections::{BTreeMap, BTreeSet};

use piton_core::{Effect, Input, JournalEntry, Session, Submission};
use piton_domain::{
    ContentBlock, EffectId, Json, Millis, ModelPolicy, ModelResponse, Offset, Record, RecordBody,
    RequestId, RetryPolicy, SCHEMA_VERSION, Safety, Timestamp, ToolName, ToolSpec, ToolUseId,
};

use crate::Violation;

/// The session's tools and model policy, and how much nondeterminism the
/// generator may inject. Not part of the explored state.
#[derive(Clone, Debug)]
pub struct WorldConfig {
    pub tools: Vec<ToolSpec>,
    pub model: ModelPolicy,
    /// Each entry is a reply the model may give: one tool use per name.
    /// Names outside `tools` exercise unknown-tool handling.
    pub tool_replies: Vec<Vec<ToolName>>,
    /// Also offer a reply that reuses a tool use id.
    pub invalid_replies: bool,
    /// How many replies may ask for tools, across the whole run.
    pub tool_rounds: u8,
    /// How many model calls may fail, across the whole run.
    pub model_failures: u8,
    pub submits: u8,
    pub cancels: u8,
    pub crashes: u8,
    /// Deliver completions for effects the session already abandoned.
    pub late_completions: bool,
    /// Let an append given up on at a crash land afterwards.
    pub late_appends: bool,
}

impl WorldConfig {
    /// Small enough for Stateright to explore every state, large enough to
    /// reach parallel tools, an unknown tool, an invalid reply, a retry, a
    /// timeout, a cancel, a late append and a crash in one run.
    #[must_use]
    pub fn exhaustive() -> Self {
        Self {
            tools: tools(),
            model: model_policy(),
            tool_replies: Vec::from([
                Vec::from([search(), send_email()]),
                Vec::from([ToolName::new("missing".to_owned())]),
            ]),
            invalid_replies: true,
            tool_rounds: 1,
            model_failures: 1,
            submits: 1,
            cancels: 1,
            crashes: 1,
            late_completions: true,
            late_appends: true,
        }
    }

    /// For replaying journals from real shells: the session's tools and
    /// model policy, and nothing for the generator to do.
    #[must_use]
    pub const fn replaying(tools: Vec<ToolSpec>, model: ModelPolicy) -> Self {
        Self {
            tools,
            model,
            tool_replies: Vec::new(),
            invalid_replies: false,
            tool_rounds: 0,
            model_failures: 0,
            submits: 0,
            cancels: 0,
            crashes: 0,
            late_completions: false,
            late_appends: false,
        }
    }

    /// Larger budgets for random walks, which go deep rather than wide.
    #[must_use]
    pub fn deep() -> Self {
        Self {
            tools: tools(),
            model: model_policy(),
            tool_replies: Vec::from([
                Vec::from([search()]),
                Vec::from([send_email()]),
                Vec::from([search(), send_email()]),
                Vec::from([ToolName::new("missing".to_owned()), search()]),
            ]),
            invalid_replies: true,
            tool_rounds: 12,
            model_failures: 6,
            submits: 8,
            cancels: 3,
            crashes: 4,
            late_completions: true,
            late_appends: true,
        }
    }
}

/// A tool that is safe to run again.
#[must_use]
pub fn search() -> ToolName {
    ToolName::new("search".to_owned())
}

/// A tool that must never run twice.
#[must_use]
pub fn send_email() -> ToolName {
    ToolName::new("send_email".to_owned())
}

/// The tools every preset gives its session.
#[must_use]
pub fn tools() -> Vec<ToolSpec> {
    Vec::from([
        ToolSpec {
            name: search(),
            safety: Safety::Idempotent,
            timeout: Millis::new(10),
            cancel_grace: Millis::ZERO,
        },
        ToolSpec {
            name: send_email(),
            safety: Safety::Unsafe,
            timeout: Millis::new(10),
            cancel_grace: Millis::new(5),
        },
    ])
}

/// The model policy every preset gives its session.
#[must_use]
pub const fn model_policy() -> ModelPolicy {
    ModelPolicy {
        timeout: Millis::new(10),
        retry: RetryPolicy {
            max_attempts: 2,
            initial_backoff: Millis::new(2),
            max_backoff: Millis::new(8),
        },
    }
}

/// Something the environment does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EnvAction {
    /// The shell hands the session an input at the current time.
    Deliver(Input),
    /// Time reaches `at` and the shell ticks the session.
    Tick { at: Timestamp },
    /// The process dies and the session restarts from its durable log. The
    /// outstanding append may or may not have reached storage. The shell
    /// does the same when an append fails, since it cannot tell whether the
    /// append landed.
    Crash { append_lands: bool },
    /// An append the shell gave up on (the `n`th still pending) reaches
    /// storage after all, unseen by the session.
    LateAppendLands(usize),
}

/// An append storage has not answered.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Pending {
    expected: Offset,
    records: Vec<Record>,
}

/// Everything outside the session, including the truth about what effects
/// really did, which survives crashes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Env {
    now: Timestamp,
    /// What storage holds.
    durable: Vec<Record>,
    /// The log length the session last learned is durable.
    acknowledged: Offset,
    in_flight: Option<Pending>,
    /// Appends from earlier lives that may still land. All expect the
    /// current tail; once it moves, none of them can.
    late: Vec<Pending>,
    late_landed: bool,
    dispatched: BTreeMap<EffectId, Effect>,
    cancel_requested: BTreeSet<EffectId>,
    /// Effects the session dropped; a completion may still be on its way.
    abandoned: BTreeMap<EffectId, Effect>,
    /// Effects that were ever dispatched, in any life.
    started: BTreeSet<EffectId>,
    /// Tools that confirmed they stopped.
    stopped: BTreeSet<EffectId>,
    submits: u8,
    cancels: u8,
    crashes: u8,
    tool_rounds: u8,
    model_failures: u8,
    /// The first rule the session broke while talking to the environment.
    violation: Option<Violation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct World {
    session: Session,
    env: Env,
}

impl World {
    /// A session whose log holds only its creation record, made at `now`.
    #[must_use]
    pub fn new(config: &WorldConfig, now: Timestamp) -> Self {
        Self::start(config, now).0
    }

    /// Like [`World::new`], also returning the journal entry for the
    /// session's start.
    ///
    /// # Panics
    ///
    /// Panics if the core rejects a freshly created log.
    #[must_use]
    pub fn start(config: &WorldConfig, now: Timestamp) -> (Self, JournalEntry) {
        let created = Record {
            offset: Offset::ZERO,
            at: now,
            body: RecordBody::SessionCreated {
                schema: SCHEMA_VERSION,
                tools: config.tools.clone(),
                model: config.model,
            },
        };
        let durable = Vec::from([created]);
        let mut session = Session::from_log(now, &durable).expect("a fresh log recovers");
        let mut env = Env {
            now,
            acknowledged: log_len(&durable),
            durable,
            in_flight: None,
            late: Vec::new(),
            late_landed: false,
            dispatched: BTreeMap::new(),
            cancel_requested: BTreeSet::new(),
            abandoned: BTreeMap::new(),
            started: BTreeSet::new(),
            stopped: BTreeSet::new(),
            submits: 0,
            cancels: 0,
            crashes: 0,
            tool_rounds: 0,
            model_failures: 0,
            violation: None,
        };
        let submissions = env.drain(&mut session);
        let entry = JournalEntry::Recovered {
            now,
            log_len: env.acknowledged,
            submissions,
        };
        (Self { session, env }, entry)
    }

    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    #[must_use]
    pub const fn env(&self) -> &Env {
        &self.env
    }

    /// Moves the environment's clock to `now`, unless it is already later.
    pub fn advance_to(&mut self, now: Timestamp) {
        self.env.now = self.env.now.max(now);
    }
}

impl Env {
    #[must_use]
    pub const fn now(&self) -> Timestamp {
        self.now
    }

    /// What storage holds, including appends the session has not learned of.
    #[must_use]
    pub fn durable(&self) -> &[Record] {
        &self.durable
    }

    /// The log as the session knows it: acknowledged records, then the
    /// outstanding append.
    pub fn known(&self) -> impl Iterator<Item = &Record> {
        let acknowledged = usize::try_from(self.acknowledged.get()).unwrap_or(usize::MAX);
        self.durable
            .iter()
            .take(acknowledged)
            .chain(self.in_flight.iter().flat_map(|pending| &pending.records))
    }

    #[must_use]
    pub const fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether `effect` was dispatched in any life of the session.
    #[must_use]
    pub fn started(&self, effect: EffectId) -> bool {
        self.started.contains(&effect)
    }

    /// Whether `effect` confirmed it stopped.
    #[must_use]
    pub fn stopped(&self, effect: EffectId) -> bool {
        self.stopped.contains(&effect)
    }

    /// Whether some tool ever confirmed it stopped.
    #[must_use]
    pub fn any_stopped(&self) -> bool {
        !self.stopped.is_empty()
    }

    #[must_use]
    pub const fn late_landed(&self) -> bool {
        self.late_landed
    }

    #[must_use]
    pub const fn violation(&self) -> Option<&Violation> {
        self.violation.as_ref()
    }

    /// Whether storage durably holds the admission of `request_id`.
    fn admitted(&self, request_id: &RequestId) -> bool {
        self.durable.iter().any(|record| {
            matches!(&record.body, RecordBody::InputAdmitted { request_id: admitted, .. } if admitted == request_id)
        })
    }

    /// Storage takes `records` at its tail. Appends still waiting to land
    /// expected the old tail, so none of them can land any more.
    fn land(&mut self, records: Vec<Record>) {
        self.durable.extend(records);
        let tail = self.tail();
        self.late.retain(|late| late.expected == tail);
    }

    fn tail(&self) -> Offset {
        log_len(&self.durable)
    }

    /// Whether the outstanding append can still land: storage only accepts
    /// it at the offset it expects.
    fn append_can_land(&self) -> bool {
        self.in_flight
            .as_ref()
            .is_some_and(|pending| pending.expected == self.tail())
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
                if *expected != self.acknowledged {
                    return Err(Violation::AppendNotAtTail {
                        expected: *expected,
                        tail: self.acknowledged,
                    });
                }
                self.in_flight = Some(Pending {
                    expected: *expected,
                    records: records.clone(),
                });
            }
            Submission::Dispatch { effect } => {
                let id = effect.id();
                let intent = (id.intent() < self.acknowledged)
                    .then(|| usize::try_from(id.intent().get()).ok())
                    .flatten()
                    .and_then(|index| self.durable.get(index))
                    .ok_or(Violation::DispatchBeforeDurable(id))?;
                let safety = match (&intent.body, effect) {
                    (
                        RecordBody::ModelIntent { request, .. },
                        Effect::CallModel {
                            request: dispatched,
                            ..
                        },
                    ) if request == dispatched => Safety::Idempotent,
                    (
                        RecordBody::ToolIntent { call, safety, .. },
                        Effect::RunTool {
                            call: dispatched, ..
                        },
                    ) if call == dispatched => *safety,
                    _ => return Err(Violation::DispatchMismatch(id)),
                };
                if self.dispatched.insert(id, effect.clone()).is_some() {
                    return Err(Violation::DoubleDispatch(id));
                }
                if !self.started.insert(id) && safety == Safety::Unsafe {
                    return Err(Violation::UnsafeRedispatch(id));
                }
            }
            Submission::Cancel { effect } => {
                if !self.dispatched.contains_key(effect) {
                    return Err(Violation::CancelUndispatched(*effect));
                }
                self.cancel_requested.insert(*effect);
            }
            Submission::Abandon { effect } => {
                let dropped = self
                    .dispatched
                    .remove(effect)
                    .ok_or(Violation::AbandonUndispatched(*effect))?;
                self.abandoned.insert(*effect, dropped);
            }
        }
        Ok(())
    }

    /// Forgets an effect whose completion the shell is delivering.
    fn finish(&mut self, effect: EffectId) {
        self.dispatched.remove(&effect);
        self.abandoned.remove(&effect);
        self.cancel_requested.remove(&effect);
    }

    /// The effect a completion for `effect` would answer, if the shell could
    /// deliver one: it is running, or was abandoned after it finished.
    fn deliverable(&self, effect: EffectId) -> Option<&Effect> {
        self.dispatched
            .get(&effect)
            .or_else(|| self.abandoned.get(&effect))
    }
}

/// What the generator explores next, in a deterministic order.
#[must_use]
pub fn enabled(config: &WorldConfig, world: &World) -> Vec<EnvAction> {
    let env = &world.env;
    let mut actions = Vec::new();
    if let Some(pending) = env.in_flight.as_ref().filter(|_| env.append_can_land()) {
        actions.push(EnvAction::Deliver(Input::Appended {
            end: end_of(pending),
        }));
    }
    for (id, effect) in &env.dispatched {
        match effect {
            Effect::CallModel { .. } => {
                actions.push(EnvAction::Deliver(respond(*id, done())));
                if env.model_failures < config.model_failures {
                    for retryable in [true, false] {
                        actions.push(EnvAction::Deliver(Input::ModelFailed {
                            effect: *id,
                            reason: "provider error".to_owned(),
                            retryable,
                            retry_after: None,
                        }));
                    }
                }
                if env.tool_rounds < config.tool_rounds {
                    for names in &config.tool_replies {
                        actions.push(EnvAction::Deliver(respond(*id, tool_reply(*id, names))));
                    }
                    if config.invalid_replies
                        && let Some(names) = config.tool_replies.first()
                    {
                        let mut reply = tool_reply(*id, names);
                        reply.content.extend(reply.content.clone());
                        actions.push(EnvAction::Deliver(respond(*id, reply)));
                    }
                }
            }
            Effect::RunTool { .. } => actions.push(EnvAction::Deliver(tool_ok(*id))),
        }
        if env.cancel_requested.contains(id) {
            actions.push(EnvAction::Deliver(Input::EffectCancelled { effect: *id }));
        }
    }
    if config.late_completions {
        for (id, effect) in &env.abandoned {
            actions.push(EnvAction::Deliver(match effect {
                Effect::CallModel { .. } => respond(*id, done()),
                Effect::RunTool { .. } => tool_ok(*id),
            }));
        }
    }
    if env.submits < config.submits {
        actions.push(EnvAction::Deliver(submit(env.submits)));
        if env.submits > 0 {
            actions.push(EnvAction::Deliver(submit(0)));
        }
    }
    // Clients resubmit until they see their input admitted in the durable
    // log, so a crash cannot lose a request for good.
    for index in 0..env.submits {
        if !env.admitted(&request_id(index)) {
            actions.push(EnvAction::Deliver(submit(index)));
        }
    }
    if env.cancels < config.cancels
        && let Some(turn) = world.session.active_turn()
    {
        actions.push(EnvAction::Deliver(Input::CancelTurn { turn }));
    }
    if env.crashes < config.crashes {
        if env.append_can_land() {
            actions.push(EnvAction::Crash { append_lands: true });
        }
        actions.push(EnvAction::Crash {
            append_lands: false,
        });
    } else if env.in_flight.is_some() && !env.append_can_land() {
        // Storage rejects the append; the shell recovers whatever the budget.
        actions.push(EnvAction::Crash {
            append_lands: false,
        });
    }
    if config.late_appends {
        for (index, late) in env.late.iter().enumerate() {
            if late.expected == env.tail() {
                actions.push(EnvAction::LateAppendLands(index));
            }
        }
    }
    if let Some(at) = world.session.deadline() {
        actions.push(EnvAction::Tick { at });
    }
    actions
}

/// Whether a shell could do `action` now, whatever its payload. This is the
/// contract real shells are held to; [`enabled`] explores a finite part of
/// it.
#[must_use]
pub fn admits(world: &World, action: &EnvAction) -> bool {
    let env = &world.env;
    let is_model = |effect: &EffectId| {
        env.deliverable(*effect)
            .is_some_and(|effect| matches!(effect, Effect::CallModel { .. }))
    };
    let is_tool = |effect: &EffectId| {
        env.deliverable(*effect)
            .is_some_and(|effect| matches!(effect, Effect::RunTool { .. }))
    };
    match action {
        EnvAction::Deliver(input) => match input {
            Input::Appended { end } => {
                env.append_can_land() && env.in_flight.as_ref().map(end_of) == Some(*end)
            }
            Input::ModelResponded { effect, .. } | Input::ModelFailed { effect, .. } => {
                is_model(effect)
            }
            Input::ToolReturned { effect, .. } => is_tool(effect),
            Input::EffectCancelled { effect } => env.cancel_requested.contains(effect),
            Input::Submit { .. } | Input::CancelTurn { .. } | Input::Tick => true,
        },
        EnvAction::Tick { .. }
        | EnvAction::Crash {
            append_lands: false,
        } => true,
        EnvAction::Crash { append_lands: true } => env.append_can_land(),
        EnvAction::LateAppendLands(index) => env
            .late
            .get(*index)
            .is_some_and(|late| late.expected == env.tail()),
    }
}

/// Does `action` to the world and returns the journal entry a shell would
/// write for it, if any: storage acting on its own is invisible to the
/// session.
pub fn step(world: &mut World, action: EnvAction) -> Option<JournalEntry> {
    let env = &mut world.env;
    let input = match action {
        EnvAction::Deliver(input) => {
            match &input {
                Input::Appended { .. } => {
                    if let Some(pending) = env.in_flight.take() {
                        env.land(pending.records);
                    }
                    env.acknowledged = env.tail();
                }
                Input::ModelResponded { effect, response } => {
                    env.finish(*effect);
                    if !response.tool_calls().is_empty() {
                        env.tool_rounds = env.tool_rounds.saturating_add(1);
                    }
                }
                Input::ModelFailed { effect, .. } => {
                    env.finish(*effect);
                    env.model_failures = env.model_failures.saturating_add(1);
                }
                Input::ToolReturned { effect, .. } => env.finish(*effect),
                Input::EffectCancelled { effect } => {
                    env.finish(*effect);
                    env.stopped.insert(*effect);
                }
                Input::Submit { request_id, .. } => {
                    if *request_id == self::request_id(env.submits) {
                        env.submits = env.submits.saturating_add(1);
                    }
                }
                Input::CancelTurn { .. } => env.cancels = env.cancels.saturating_add(1),
                Input::Tick => {}
            }
            input
        }
        EnvAction::Tick { at } => {
            env.now = env.now.max(at);
            Input::Tick
        }
        EnvAction::Crash { append_lands } => {
            env.crashes = env.crashes.saturating_add(1);
            if let Some(pending) = env.in_flight.take()
                && pending.expected == env.tail()
            {
                if append_lands {
                    env.land(pending.records);
                } else {
                    env.late.push(pending);
                }
            }
            env.dispatched.clear();
            env.cancel_requested.clear();
            env.abandoned.clear();
            env.acknowledged = env.tail();
            match Session::from_log(env.now, &env.durable) {
                Ok(session) => world.session = session,
                Err(error) => {
                    env.violation
                        .get_or_insert(Violation::Unrecoverable(error.to_string()));
                }
            }
            let submissions = env.drain(&mut world.session);
            return Some(JournalEntry::Recovered {
                now: env.now,
                log_len: env.acknowledged,
                submissions,
            });
        }
        EnvAction::LateAppendLands(index) => {
            if index < env.late.len() {
                let late = env.late.swap_remove(index);
                if late.expected == env.tail() {
                    env.land(late.records);
                    env.late_landed = true;
                }
            }
            return None;
        }
    };
    world.session.handle(env.now, input.clone());
    let submissions = env.drain(&mut world.session);
    Some(JournalEntry::Handled {
        now: env.now,
        input,
        submissions,
    })
}

/// Every sequence of actions that explains a restart from `log_len` records.
///
/// The outstanding append landed, a late append landed first, or nothing
/// landed. More than one can fit, since a journal records only the length.
#[must_use]
pub fn recoveries(world: &World, log_len: Offset) -> Vec<Vec<EnvAction>> {
    let env = &world.env;
    let tail = env.tail();
    let lands = |pending: &Pending| {
        pending.expected == tail
            && u64::try_from(pending.records.len())
                .ok()
                .and_then(|added| tail.get().checked_add(added))
                == Some(log_len.get())
    };
    let mut explanations = Vec::new();
    if env.in_flight.as_ref().is_some_and(lands) {
        explanations.push(Vec::from([EnvAction::Crash { append_lands: true }]));
    }
    for (index, late) in env.late.iter().enumerate() {
        if lands(late) {
            explanations.push(Vec::from([
                EnvAction::LateAppendLands(index),
                EnvAction::Crash {
                    append_lands: false,
                },
            ]));
        }
    }
    if log_len == tail {
        explanations.push(Vec::from([EnvAction::Crash {
            append_lands: false,
        }]));
    }
    explanations
}

fn end_of(pending: &Pending) -> Offset {
    let added = u64::try_from(pending.records.len()).expect("an append's length fits in u64");
    Offset::new(
        pending
            .expected
            .get()
            .checked_add(added)
            .expect("a log length fits in u64"),
    )
}

fn log_len(records: &[Record]) -> Offset {
    Offset::new(u64::try_from(records.len()).expect("a log length fits in u64"))
}

fn request_id(index: u8) -> RequestId {
    RequestId::new(format!("request-{index}"))
}

fn submit(index: u8) -> Input {
    Input::Submit {
        request_id: request_id(index),
        text: "hello".to_owned(),
    }
}

const fn respond(effect: EffectId, response: ModelResponse) -> Input {
    Input::ModelResponded { effect, response }
}

fn done() -> ModelResponse {
    ModelResponse {
        content: Vec::from([ContentBlock::Text {
            text: "done".to_owned(),
        }]),
    }
}

fn tool_ok(effect: EffectId) -> Input {
    Input::ToolReturned {
        effect,
        output: Json::string("ok"),
        is_error: false,
    }
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
