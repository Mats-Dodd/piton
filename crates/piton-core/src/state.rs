use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use core::fmt;

use piton_domain::{
    EffectId, ModelOutcome, Offset, Record, RecordBody, ReplayPolicy, RequestId, SCHEMA_VERSION,
    ToolCall, ToolName, ToolOutcome, Transcript, TurnEnd, TurnId,
};

use crate::Effect;

/// Everything a session knows that derives from its log. Built only by
/// folding records, so a recovered session and a live one that produced the
/// same records hold equal state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionState {
    /// The offset the next record must carry.
    next: Offset,
    tools: BTreeMap<ToolName, ReplayPolicy>,
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
    /// Offset of the `TurnCancelled` record, once there is one.
    cancelled: Option<Offset>,
    phase: Phase,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Phase {
    CallModel,
    AwaitModel,
    /// The model asked for these tools; their intents are not logged yet.
    IntendTools(VecDeque<ToolCall>),
    AwaitTools,
    End(TurnEnd),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpenEffect {
    pub(crate) turn: TurnId,
    pub(crate) effect: Effect,
    pub(crate) replay: ReplayPolicy,
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
        let RecordBody::SessionCreated { schema, tools } = &record.body else {
            return Err(RecoveryError::NotCreated);
        };
        if *schema != SCHEMA_VERSION {
            return Err(RecoveryError::UnsupportedSchema(*schema));
        }
        Ok(Self {
            next: record.offset.next(),
            tools: tools
                .iter()
                .map(|spec| (spec.name.clone(), spec.replay))
                .collect(),
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
            RecordBody::InputAdmitted { request_id, .. } => {
                self.admitted.insert(request_id.clone());
                self.queue.push_back(request_id.clone());
            }
            RecordBody::TurnStarted { request_id } => {
                self.queue.retain(|queued| queued != request_id);
                self.turn = Some(Turn {
                    id: TurnId::new(record.offset),
                    cancelled: None,
                    phase: Phase::CallModel,
                });
            }
            RecordBody::ModelIntent { turn, request } => {
                self.open.insert(
                    id,
                    OpenEffect {
                        turn: *turn,
                        effect: Effect::CallModel {
                            id,
                            request: request.clone(),
                        },
                        replay: ReplayPolicy::Redispatch,
                    },
                );
                if let Some(current) = self.current_turn_mut(*turn) {
                    current.phase = Phase::AwaitModel;
                }
            }
            RecordBody::ModelResolved { effect, outcome } => {
                let turn = self.open.remove(effect).map(|open| open.turn);
                if let Some(current) = turn.and_then(|turn| self.current_turn_mut(turn))
                    && current.phase == Phase::AwaitModel
                {
                    current.phase = match outcome {
                        ModelOutcome::Responded { response } => {
                            let calls = response.tool_calls();
                            if calls.is_empty() {
                                Phase::End(TurnEnd::Completed)
                            } else {
                                Phase::IntendTools(calls.into())
                            }
                        }
                        ModelOutcome::Failed { .. } => Phase::End(TurnEnd::Failed),
                        ModelOutcome::Cancelled => Phase::End(TurnEnd::Cancelled),
                    };
                }
            }
            RecordBody::ToolIntent { turn, call, replay } => {
                self.open.insert(
                    id,
                    OpenEffect {
                        turn: *turn,
                        effect: Effect::RunTool {
                            id,
                            call: call.clone(),
                        },
                        replay: *replay,
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
                    current.cancelled = Some(record.offset);
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
        if turn.phase == Phase::CallModel && turn.cancelled.is_some() {
            turn.phase = Phase::End(TurnEnd::Cancelled);
        }
    }

    /// The next record the session should produce, if any. Each record it
    /// returns moves the state on, so calling this until `None` terminates.
    pub(crate) fn next_record(&self, dispatched: &BTreeSet<EffectId>) -> Option<RecordBody> {
        let Some(turn) = &self.turn else {
            return self
                .queue
                .front()
                .map(|request_id| RecordBody::TurnStarted {
                    request_id: request_id.clone(),
                });
        };
        if turn.cancelled.is_some()
            && let Some((id, open)) = self
                .open
                .iter()
                .find(|(id, open)| open.turn == turn.id && !dispatched.contains(id))
        {
            return Some(resolution(*id, open, ToolOutcome::Cancelled));
        }
        if let Some((id, _)) = self
            .open
            .iter()
            .find(|(_, open)| !self.is_runnable(&open.effect))
        {
            return Some(RecordBody::ToolResolved {
                effect: *id,
                outcome: ToolOutcome::UnknownTool,
            });
        }
        match &turn.phase {
            Phase::CallModel => Some(RecordBody::ModelIntent {
                turn: turn.id,
                request: self.transcript.request(),
            }),
            Phase::IntendTools(calls) => calls.front().map(|call| RecordBody::ToolIntent {
                turn: turn.id,
                call: call.clone(),
                replay: self
                    .tools
                    .get(&call.name)
                    .copied()
                    .unwrap_or(ReplayPolicy::Interrupt),
            }),
            Phase::End(reason) => Some(RecordBody::TurnEnded {
                turn: turn.id,
                reason: *reason,
            }),
            Phase::AwaitModel | Phase::AwaitTools => None,
        }
    }

    /// Model calls always run; tool calls run only if the session has the tool.
    fn is_runnable(&self, effect: &Effect) -> bool {
        match effect {
            Effect::CallModel { .. } => true,
            Effect::RunTool { call, .. } => self.tools.contains_key(&call.name),
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

    /// Offset of the active turn's `TurnCancelled` record, if it has one.
    pub(crate) fn cancelled_at(&self) -> Option<Offset> {
        self.turn.as_ref().and_then(|turn| turn.cancelled)
    }

    pub(crate) fn is_cancelled(&self, turn: TurnId) -> bool {
        self.turn
            .as_ref()
            .is_some_and(|current| current.id == turn && current.cancelled.is_some())
    }

    /// Open tool effects recovery must not run again: they may have run
    /// already and their policy forbids a repeat.
    pub(crate) fn interrupted_by_restart(&self) -> impl Iterator<Item = EffectId> {
        self.open
            .iter()
            .filter(|(_, open)| {
                open.replay == ReplayPolicy::Interrupt && self.is_runnable(&open.effect)
            })
            .filter(|(_, open)| matches!(open.effect, Effect::RunTool { .. }))
            .map(|(id, _)| *id)
    }
}

/// The record that resolves `open` without the effect having produced output:
/// tools take `outcome`, model calls count as cancelled.
pub(crate) fn resolution(id: EffectId, open: &OpenEffect, outcome: ToolOutcome) -> RecordBody {
    match open.effect {
        Effect::CallModel { .. } => RecordBody::ModelResolved {
            effect: id,
            outcome: ModelOutcome::Cancelled,
        },
        Effect::RunTool { .. } => RecordBody::ToolResolved {
            effect: id,
            outcome,
        },
    }
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
