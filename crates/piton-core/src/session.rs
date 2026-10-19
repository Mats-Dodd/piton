use alloc::{
    collections::{BTreeSet, VecDeque},
    string::ToString as _,
    vec::Vec,
};

use piton_domain::{
    EffectId, ModelOutcome, Offset, Record, RecordBody, Timestamp, ToolOutcome, TurnId,
};

use crate::{
    Effect, Input, Submission,
    state::{RecoveryError, SessionState, resolution},
};

/// One agent session: a deterministic state machine over its log.
///
/// Drive it by calling [`Session::handle`] and then draining
/// [`Session::poll_submission`] until it returns `None`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Session {
    state: SessionState,
    /// Log length storage has acknowledged.
    durable: Offset,
    /// End of the outstanding append, if there is one.
    in_flight: Option<Offset>,
    /// Records produced but not yet handed to storage.
    unsubmitted: Vec<Record>,
    /// Effects handed to the shell that have not completed.
    dispatched: BTreeSet<EffectId>,
    /// Dispatched effects the shell has been asked to cancel.
    cancelling: BTreeSet<EffectId>,
    outbox: VecDeque<Submission>,
    now: Timestamp,
}

impl Session {
    /// Recovers a session from its durable log.
    ///
    /// Tool calls that may have run before the restart and must not repeat
    /// are resolved as interrupted; everything else carries on, and open
    /// effects are dispatched again under their original ids.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] when `records` is not a well-formed log.
    pub fn from_log(now: Timestamp, records: &[Record]) -> Result<Self, RecoveryError> {
        let state = SessionState::fold(records)?;
        let last_at = records.last().map_or(Timestamp::EPOCH, |record| record.at);
        let interrupted: Vec<EffectId> = state.interrupted_by_restart().collect();
        let mut session = Self {
            durable: state.next_offset(),
            state,
            in_flight: None,
            unsubmitted: Vec::new(),
            dispatched: BTreeSet::new(),
            cancelling: BTreeSet::new(),
            outbox: VecDeque::new(),
            now: now.max(last_at),
        };
        for effect in interrupted {
            session.produce(RecordBody::ToolResolved {
                effect,
                outcome: ToolOutcome::Interrupted,
            });
        }
        session.drive();
        Ok(session)
    }

    /// Feeds one input to the session. `now` never moves the session's clock
    /// backwards.
    ///
    /// # Panics
    ///
    /// Panics if the shell acknowledges an append that is not outstanding.
    pub fn handle(&mut self, now: Timestamp, input: Input) {
        self.now = self.now.max(now);
        match input {
            Input::Appended { end } => {
                assert_eq!(
                    self.in_flight,
                    Some(end),
                    "an acknowledgement must match the outstanding append"
                );
                self.durable = end;
                self.in_flight = None;
            }
            Input::ModelResponded { effect, response } => {
                if let Some(turn) = self.complete(effect, true) {
                    let outcome = if self.state.is_cancelled(turn) {
                        ModelOutcome::Cancelled
                    } else {
                        match response.validate() {
                            Ok(()) => ModelOutcome::Responded { response },
                            Err(invalid) => ModelOutcome::Failed {
                                reason: invalid.to_string(),
                            },
                        }
                    };
                    self.produce(RecordBody::ModelResolved { effect, outcome });
                }
            }
            Input::ModelFailed { effect, reason } => {
                if self.complete(effect, true).is_some() {
                    self.produce(RecordBody::ModelResolved {
                        effect,
                        outcome: ModelOutcome::Failed { reason },
                    });
                }
            }
            Input::ToolReturned {
                effect,
                output,
                is_error,
            } => {
                if self.complete(effect, false).is_some() {
                    self.produce(RecordBody::ToolResolved {
                        effect,
                        outcome: ToolOutcome::Returned { output, is_error },
                    });
                }
            }
            Input::EffectCancelled { effect } => {
                let is_model = self.is_model_call(effect);
                if self.complete(effect, is_model).is_some()
                    && let Some(open) = self.state.open_effects().get(&effect)
                {
                    let body = resolution(effect, open, ToolOutcome::Cancelled);
                    self.produce(body);
                }
            }
            Input::Submit { request_id, text } => {
                if !self.state.is_admitted(&request_id) {
                    self.produce(RecordBody::InputAdmitted { request_id, text });
                }
            }
            Input::CancelTurn { turn } => {
                if self.state.active_turn() == Some(turn) && !self.state.is_cancelled(turn) {
                    self.produce(RecordBody::TurnCancelled { turn });
                }
            }
        }
        self.drive();
    }

    /// The next piece of work for the shell.
    pub fn poll_submission(&mut self) -> Option<Submission> {
        self.outbox.pop_front()
    }

    /// When the session next needs to hear from the shell even without input.
    /// Nothing is time-driven yet.
    #[must_use]
    pub const fn deadline(&self) -> Option<Timestamp> {
        None
    }

    #[must_use]
    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    /// Records produced but not yet handed to storage.
    #[must_use]
    pub fn unsubmitted(&self) -> &[Record] {
        &self.unsubmitted
    }

    /// Log length storage has acknowledged.
    #[must_use]
    pub const fn durable_len(&self) -> Offset {
        self.durable
    }

    /// Log length including records storage has not acknowledged.
    #[must_use]
    pub const fn log_len(&self) -> Offset {
        self.state.next_offset()
    }

    #[must_use]
    pub fn active_turn(&self) -> Option<TurnId> {
        self.state.active_turn()
    }

    /// Accepts a completion for `effect` if it is outstanding and of the
    /// expected kind, returning its turn. Anything else is stale and ignored.
    fn complete(&mut self, effect: EffectId, is_model: bool) -> Option<TurnId> {
        let open = self.state.open_effects().get(&effect)?;
        if matches!(open.effect, Effect::CallModel { .. }) != is_model
            || !self.dispatched.remove(&effect)
        {
            return None;
        }
        self.cancelling.remove(&effect);
        Some(open.turn)
    }

    fn is_model_call(&self, effect: EffectId) -> bool {
        self.state
            .open_effects()
            .get(&effect)
            .is_some_and(|open| matches!(open.effect, Effect::CallModel { .. }))
    }

    fn produce(&mut self, body: RecordBody) {
        let record = Record {
            offset: self.state.next_offset(),
            at: self.now,
            body,
        };
        self.state
            .apply(&record)
            .expect("a session only produces records that extend its own log");
        self.unsubmitted.push(record);
    }

    /// Produces every record the state calls for, then hands the shell what
    /// is now safe to do.
    fn drive(&mut self) {
        while let Some(body) = self.state.next_record(&self.dispatched) {
            self.produce(body);
        }
        self.dispatch_durable_intents();
        self.cancel_dispatched();
        self.flush();
    }

    fn dispatch_durable_intents(&mut self) {
        let ready: Vec<Effect> = self
            .state
            .open_effects()
            .iter()
            .filter(|(id, open)| {
                let durable = cfg!(piton_planted_bug) || id.intent() < self.durable;
                durable && !self.dispatched.contains(id) && !self.state.is_cancelled(open.turn)
            })
            .map(|(_, open)| open.effect.clone())
            .collect();
        for effect in ready {
            self.dispatched.insert(effect.id());
            self.outbox.push_back(Submission::Dispatch { effect });
        }
    }

    /// Once a cancellation is durable, asks the shell to stop the turn's
    /// dispatched effects.
    fn cancel_dispatched(&mut self) {
        let Some(cancelled_at) = self.state.cancelled_at() else {
            return;
        };
        if cancelled_at >= self.durable {
            return;
        }
        let to_cancel: Vec<EffectId> = self
            .dispatched
            .difference(&self.cancelling)
            .copied()
            .collect();
        for effect in to_cancel {
            self.cancelling.insert(effect);
            self.outbox.push_back(Submission::Cancel { effect });
        }
    }

    fn flush(&mut self) {
        if self.in_flight.is_some() || self.unsubmitted.is_empty() {
            return;
        }
        self.in_flight = Some(self.state.next_offset());
        self.outbox.push_back(Submission::Append {
            expected: self.durable,
            records: core::mem::take(&mut self.unsubmitted),
        });
    }
}
