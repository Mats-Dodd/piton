use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use core::fmt;

use piton_domain::{
    EffectId, Millis, ModelOutcome, ModelPolicy, Offset, Record, RecordBody, RequestId,
    SCHEMA_VERSION, Safety, Timestamp, ToolCall, ToolName, ToolSpec, Transcript, TurnEnd, TurnId,
};

use crate::Effect;

/// Everything a session knows that derives from its log. Built only by
/// folding records, so a recovered session and a live one that produced the
/// same records hold equal state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionState {
    /// The offset the next record must carry.
    next: Offset,
    tools: BTreeMap<ToolName, ToolSpec>,
    model: ModelPolicy,
    admitted: BTreeSet<RequestId>,
    queue: VecDeque<RequestId>,
    turn: Option<Turn>,
    /// Effects whose intent is logged but whose outcome is not.
    open: BTreeMap<EffectId, OpenEffect>,
    transcript: Transcript,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Turn {
    id: TurnId,
    /// When the turn was cancelled, once it has been.
    cancelled: Option<Timestamp>,
    /// Consecutive failed model calls.
    failures: u32,
    phase: Phase,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Phase {
    CallModel,
    /// A model call failed; call again once `until` has passed.
    Backoff {
        until: Timestamp,
    },
    AwaitModel,
    /// The model asked for these tools; their intents are not logged yet.
    IntendTools(VecDeque<ToolCall>),
    AwaitTools,
    End(TurnEnd),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpenEffect {
    pub(crate) turn: TurnId,
    /// When the intent was recorded; the effect's timers run from here.
    pub(crate) at: Timestamp,
    pub(crate) effect: Effect,
    pub(crate) kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Model {
        timeout: Millis,
    },
    Tool {
        safety: Safety,
        timeout: Millis,
        grace: Millis,
    },
    /// A tool the session does not have; resolved without running.
    UnknownTool,
}

/// When an open effect's time runs out, as far as the log can say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timers {
    /// An unsafe tool's timeout: past it, the tool is asked to stop (or, if
    /// it never started, is not run). Its grace period then runs from when
    /// it is asked, which only the live session knows.
    pub(crate) soft: Option<Timestamp>,
    /// Past it, the session resolves the effect without waiting any longer.
    pub(crate) hard: Option<Timestamp>,
}

impl OpenEffect {
    /// When the effect runs out of time, as far as the log can say.
    pub(crate) const fn timers(&self) -> Timers {
        match self.kind {
            Kind::Model { timeout }
            | Kind::Tool {
                safety: Safety::Idempotent,
                timeout,
                ..
            } => Timers {
                soft: None,
                hard: Some(self.at.after(timeout)),
            },
            Kind::Tool {
                safety: Safety::Unsafe,
                timeout,
                ..
            } => Timers {
                soft: Some(self.at.after(timeout)),
                hard: None,
            },
            Kind::UnknownTool => Timers {
                soft: None,
                hard: None,
            },
        }
    }
}

impl SessionState {
    /// Folds a log from its first record.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] when the records are not a well-formed log.
    pub fn fold<'record, R>(records: R) -> Result<Self, RecoveryError>
    where
        R: IntoIterator<Item = &'record Record>,
    {
        let mut records = records.into_iter();
        let first = records.next().ok_or(RecoveryError::Empty)?;
        let mut state = Self::created(first)?;
        for record in records {
            state.apply(record)?;
        }
        Ok(state)
    }

    fn created(record: &Record) -> Result<Self, RecoveryError> {
        if record.offset != Offset::ZERO {
            return Err(RecoveryError::OutOfOrder {
                expected: Offset::ZERO,
                found: record.offset,
            });
        }
        let RecordBody::SessionCreated {
            schema,
            tools,
            model,
        } = &record.body
        else {
            return Err(RecoveryError::NotCreated);
        };
        if *schema != SCHEMA_VERSION {
            return Err(RecoveryError::UnsupportedSchema(*schema));
        }
        Ok(Self {
            next: record.offset.next(),
            tools: tools
                .iter()
                .map(|spec| (spec.name.clone(), spec.clone()))
                .collect(),
            model: *model,
            admitted: BTreeSet::new(),
            queue: VecDeque::new(),
            turn: None,
            open: BTreeMap::new(),
            transcript: Transcript::fold([record]),
        })
    }

    pub(crate) fn apply(&mut self, record: &Record) -> Result<(), RecoveryError> {
        if record.offset != self.next {
            return Err(RecoveryError::OutOfOrder {
                expected: self.next,
                found: record.offset,
            });
        }
        let id = EffectId::new(record.offset);
        match &record.body {
            RecordBody::SessionCreated { .. } => return Err(RecoveryError::DuplicateCreation),
            RecordBody::SessionResumed => {}
            RecordBody::InputAdmitted { request_id, .. } => {
                self.admitted.insert(request_id.clone());
                self.queue.push_back(request_id.clone());
            }
            RecordBody::TurnStarted { request_id } => {
                self.queue.retain(|queued| queued != request_id);
                self.turn = Some(Turn {
                    id: TurnId::new(record.offset),
                    cancelled: None,
                    failures: 0,
                    phase: Phase::CallModel,
                });
            }
            RecordBody::ModelIntent { turn, request } => {
                self.open.insert(
                    id,
                    OpenEffect {
                        turn: *turn,
                        at: record.at,
                        effect: Effect::CallModel {
                            id,
                            request: request.clone(),
                        },
                        kind: Kind::Model {
                            timeout: self.model.timeout,
                        },
                    },
                );
                if let Some(current) = self.current_turn_mut(*turn) {
                    current.phase = Phase::AwaitModel;
                }
            }
            RecordBody::ModelResolved { effect, outcome } => {
                let turn = self.open.remove(effect).map(|open| open.turn);
                let retry = self.model.retry;
                if let Some(current) = turn.and_then(|turn| self.current_turn_mut(turn))
                    && current.phase == Phase::AwaitModel
                {
                    current.phase = match outcome {
                        ModelOutcome::Responded { response } => {
                            current.failures = 0;
                            let calls = response.tool_calls();
                            if calls.is_empty() {
                                Phase::End(TurnEnd::Completed)
                            } else {
                                Phase::IntendTools(calls.into())
                            }
                        }
                        ModelOutcome::Failed {
                            retryable,
                            retry_after,
                            ..
                        } => {
                            current.failures = current.failures.saturating_add(1);
                            if *retryable && current.failures < retry.max_attempts {
                                let wait = jittered(retry.backoff(current.failures), *effect)
                                    .max(retry_after.unwrap_or(Millis::ZERO));
                                Phase::Backoff {
                                    until: record.at.after(wait),
                                }
                            } else {
                                Phase::End(TurnEnd::Failed)
                            }
                        }
                        ModelOutcome::Cancelled => Phase::End(TurnEnd::Cancelled),
                    };
                }
            }
            RecordBody::ToolIntent { turn, call, .. } => {
                let kind = self
                    .tools
                    .get(&call.name)
                    .map_or(Kind::UnknownTool, |spec| Kind::Tool {
                        safety: spec.safety,
                        timeout: spec.timeout,
                        grace: spec.cancel_grace,
                    });
                self.open.insert(
                    id,
                    OpenEffect {
                        turn: *turn,
                        at: record.at,
                        effect: Effect::RunTool {
                            id,
                            call: call.clone(),
                        },
                        kind,
                    },
                );
                if let Some(current) = self.current_turn_mut(*turn)
                    && let Phase::IntendTools(calls) = &mut current.phase
                {
                    calls.pop_front();
                    if calls.is_empty() {
                        current.phase = Phase::AwaitTools;
                    }
                }
            }
            RecordBody::ToolResolved { effect, .. } => {
                self.open.remove(effect);
            }
            RecordBody::TurnCancelled { turn } => {
                if let Some(current) = self.current_turn_mut(*turn) {
                    current.cancelled = Some(record.at);
                }
            }
            RecordBody::TurnEnded { .. } => self.turn = None,
        }
        self.transcript.apply(record);
        self.next = record.offset.next();
        self.settle();
        Ok(())
    }

    /// Advances phases that need no new record to move on.
    fn settle(&mut self) {
        let Some(turn) = &mut self.turn else { return };
        let turn_has_open_effects = self.open.values().any(|open| open.turn == turn.id);
        if turn.phase == Phase::AwaitTools && !turn_has_open_effects {
            turn.phase = Phase::CallModel;
        }
        if matches!(turn.phase, Phase::CallModel | Phase::Backoff { .. })
            && turn.cancelled.is_some()
        {
            turn.phase = Phase::End(TurnEnd::Cancelled);
        }
    }

    /// The next record that moves the active turn on at `now`, if any.
    /// Resolving effects is the session's job; this covers everything else.
    /// Each record it returns changes the phase, so calling it until `None`
    /// terminates.
    pub(crate) fn next_record(&self, now: Timestamp) -> Option<RecordBody> {
        let Some(turn) = &self.turn else {
            return self
                .queue
                .front()
                .map(|request_id| RecordBody::TurnStarted {
                    request_id: request_id.clone(),
                });
        };
        match &turn.phase {
            Phase::CallModel => Some(self.model_intent(turn.id)),
            Phase::Backoff { until } if now >= *until => Some(self.model_intent(turn.id)),
            Phase::IntendTools(calls) => calls.front().map(|call| RecordBody::ToolIntent {
                turn: turn.id,
                call: call.clone(),
                safety: self
                    .tools
                    .get(&call.name)
                    .map_or(Safety::Unsafe, |spec| spec.safety),
            }),
            Phase::End(reason) => Some(RecordBody::TurnEnded {
                turn: turn.id,
                reason: *reason,
            }),
            Phase::Backoff { .. } | Phase::AwaitModel | Phase::AwaitTools => None,
        }
    }

    fn model_intent(&self, turn: TurnId) -> RecordBody {
        RecordBody::ModelIntent {
            turn,
            request: self.transcript.request(),
        }
    }

    fn current_turn_mut(&mut self, id: TurnId) -> Option<&mut Turn> {
        self.turn.as_mut().filter(|turn| turn.id == id)
    }

    pub(crate) const fn next_offset(&self) -> Offset {
        self.next
    }

    pub(crate) const fn open_effects(&self) -> &BTreeMap<EffectId, OpenEffect> {
        &self.open
    }

    pub(crate) fn is_admitted(&self, request_id: &RequestId) -> bool {
        self.admitted.contains(request_id)
    }

    pub(crate) fn active_turn(&self) -> Option<TurnId> {
        self.turn.as_ref().map(|turn| turn.id)
    }

    /// When `turn` was cancelled, if it is the active turn and has been.
    pub(crate) fn cancelled_at(&self, turn: TurnId) -> Option<Timestamp> {
        self.turn
            .as_ref()
            .filter(|current| current.id == turn)
            .and_then(|current| current.cancelled)
    }

    /// When the active turn calls the model again after a failure.
    pub(crate) fn backoff_until(&self) -> Option<Timestamp> {
        match self.turn.as_ref().map(|turn| &turn.phase) {
            Some(Phase::Backoff { until }) => Some(*until),
            Some(
                Phase::CallModel
                | Phase::AwaitModel
                | Phase::IntendTools(_)
                | Phase::AwaitTools
                | Phase::End(_),
            )
            | None => None,
        }
    }

    /// Open tools recovery must not run again: they may have run already and
    /// are unsafe to repeat.
    pub(crate) fn unsafe_tools(&self) -> impl Iterator<Item = EffectId> {
        self.open
            .iter()
            .filter(|(_, open)| {
                matches!(
                    open.kind,
                    Kind::Tool {
                        safety: Safety::Unsafe,
                        ..
                    }
                )
            })
            .map(|(id, _)| *id)
    }
}

/// Spreads retries from many sessions apart without a random number source:
/// up to a quarter of `wait` more, derived from the effect id.
fn jittered(wait: Millis, effect: EffectId) -> Millis {
    let spread = wait.get().checked_shr(2).unwrap_or(0).saturating_add(1);
    let extra = mix(effect.intent().get()).checked_rem(spread).unwrap_or(0);
    Millis::new(wait.get().saturating_add(extra))
}

/// The splitmix64 finaliser: a cheap, well-mixed hash of one integer.
const fn mix(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mixed = (mixed ^ (mixed >> 30u32)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27u32)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31u32)
}

/// Why records could not be folded into a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Empty,
    NotCreated,
    UnsupportedSchema(u32),
    DuplicateCreation,
    OutOfOrder { expected: Offset, found: Offset },
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("log is empty"),
            Self::NotCreated => formatter.write_str("log does not start with SessionCreated"),
            Self::UnsupportedSchema(schema) => write!(formatter, "unsupported log schema {schema}"),
            Self::DuplicateCreation => formatter.write_str("log has a second SessionCreated"),
            Self::OutOfOrder { expected, found } => write!(
                formatter,
                "expected a record at offset {}, found offset {}",
                expected.get(),
                found.get()
            ),
        }
    }
}

impl core::error::Error for RecoveryError {}
