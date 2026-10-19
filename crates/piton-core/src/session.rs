use alloc::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    format,
    string::{String, ToString as _},
    vec::Vec,
};

use piton_domain::{
    EffectId, ModelOutcome, Offset, Record, RecordBody, Safety, StopReason, Timestamp, ToolOutcome,
    TurnId, UnknownCause,
};

use crate::{
    Effect, Input, Submission,
    state::{Kind, OpenEffect, RecoveryError, SessionState},
};

/// One agent session: a deterministic state machine over its log.
///
/// Drive it by calling [`Session::handle`] and then draining
/// [`Session::poll_submission`] until it returns `None`. When
/// [`Session::deadline`] passes, send [`Input::Tick`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Session {
    state: SessionState,
    /// Log length storage has acknowledged.
    durable: Offset,
    /// Log length this session recovered from. Effects declared before it
    /// may have started in an earlier life of the session.
    recovered: Offset,
    /// End of the outstanding append, if there is one.
    in_flight: Option<Offset>,
    /// Records produced but not yet handed to storage.
    unsubmitted: Vec<Record>,
    /// Effects handed to the shell that have not completed.
    dispatched: BTreeSet<EffectId>,
    /// Dispatched tools the shell has been asked to stop, and when. Their
    /// grace period runs from then, so a session that notices a timeout
    /// late still gives the tool its full grace.
    cancelling: BTreeMap<EffectId, Timestamp>,
    outbox: VecDeque<Submission>,
    now: Timestamp,
}

impl Session {
    /// Recovers a session from its durable log, starting a new life of it.
    ///
    /// The session first appends `SessionResumed` and dispatches nothing
    /// until that append is durable: an append from an earlier life could
    /// otherwise land later and contradict what this life does. Unsafe tools
    /// that may have run before the restart are resolved as unknown; every
    /// other open effect is dispatched again under its original id.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] when `records` is not a well-formed log.
    pub fn from_log(now: Timestamp, records: &[Record]) -> Result<Self, RecoveryError> {
        let state = SessionState::fold(records)?;
        let last_at = records.last().map_or(Timestamp::EPOCH, |record| record.at);
        let restarted: Vec<EffectId> = state.unsafe_tools().collect();
        let mut session = Self {
            durable: state.next_offset(),
            recovered: state.next_offset(),
            state,
            in_flight: None,
            unsubmitted: Vec::new(),
            dispatched: BTreeSet::new(),
            cancelling: BTreeMap::new(),
            outbox: VecDeque::new(),
            now: now.max(last_at),
        };
        session.produce(RecordBody::SessionResumed);
        for effect in restarted {
            session.produce(RecordBody::ToolResolved {
                effect,
                outcome: ToolOutcome::Unknown {
                    cause: UnknownCause::Restart,
                },
            });
        }
        session.drive();
        Ok(session)
    }

    /// Feeds one input to the session at `now`. Timers due by `now` fire
    /// first, so the outcome never depends on whether the shell noticed the
    /// time or the input first. `now` never moves the session's clock
    /// backwards.
    pub fn handle(&mut self, now: Timestamp, input: Input) {
        self.now = self.now.max(now);
        self.produce_due();
        match input {
            // An acknowledgement for anything but the outstanding append is a
            // duplicate from storage that delivers at least once; ignore it.
            Input::Appended { end } => {
                if self.in_flight == Some(end) {
                    self.durable = end;
                    self.in_flight = None;
                }
            }
            Input::ModelResponded { effect, response } => {
                if self.complete(effect, true) {
                    // A reply that reuses a tool use id from earlier in the
                    // conversation would make every later request invalid.
                    let reused = response
                        .tool_calls()
                        .into_iter()
                        .find(|call| self.state.has_tool_use(&call.tool_use_id));
                    let outcome = match (response.validate(), reused) {
                        (Ok(()), None) => ModelOutcome::Responded { response },
                        (Err(invalid), _) => ModelOutcome::Failed {
                            reason: invalid.to_string(),
                            retryable: false,
                            retry_after: None,
                        },
                        (Ok(()), Some(call)) => ModelOutcome::Failed {
                            reason: format!(
                                "response reuses tool use id {}",
                                call.tool_use_id.as_str()
                            ),
                            retryable: false,
                            retry_after: None,
                        },
                    };
                    self.produce(RecordBody::ModelResolved { effect, outcome });
                }
            }
            Input::ModelFailed {
                effect,
                reason,
                retryable,
                retry_after,
            } => {
                if self.complete(effect, true) {
                    self.produce(RecordBody::ModelResolved {
                        effect,
                        outcome: ModelOutcome::Failed {
                            reason,
                            retryable,
                            retry_after,
                        },
                    });
                }
            }
            Input::ToolReturned {
                effect,
                output,
                is_error,
            } => {
                if self.complete(effect, false) {
                    self.produce(RecordBody::ToolResolved {
                        effect,
                        outcome: ToolOutcome::Returned { output, is_error },
                    });
                }
            }
            Input::EffectCancelled { effect } => {
                let reason = self.stop_reason(effect);
                if self.cancelling.contains_key(&effect) && self.complete(effect, false) {
                    self.produce(RecordBody::ToolResolved {
                        effect,
                        outcome: ToolOutcome::Stopped { reason },
                    });
                }
            }
            Input::Submit { request_id, text } => {
                if !self.state.is_admitted(&request_id) {
                    self.produce(RecordBody::InputAdmitted { request_id, text });
                }
            }
            Input::CancelTurn { turn } => {
                if self.state.active_turn() == Some(turn) && self.state.cancelled_at(turn).is_none()
                {
                    self.produce(RecordBody::TurnCancelled { turn });
                }
            }
            Input::Tick => {}
        }
        self.drive();
    }

    /// The next piece of work for the shell.
    pub fn poll_submission(&mut self) -> Option<Submission> {
        self.outbox.pop_front()
    }

    /// When the session next needs to hear from the shell even if nothing
    /// else happens: the earliest timeout, grace period or retry backoff.
    /// Always later than the session's clock.
    #[must_use]
    pub fn deadline(&self) -> Option<Timestamp> {
        let mut earliest = self.state.backoff_until();
        for (id, open) in self.state.open_effects() {
            let timers = open.timers();
            let soft = timers.soft.filter(|_| !self.cancelling.contains_key(id));
            let abandon = self.abandon_at(*id, open);
            for deadline in [soft, timers.hard, abandon].into_iter().flatten() {
                earliest = Some(earliest.map_or(deadline, |current| current.min(deadline)));
            }
        }
        earliest
    }

    /// The session's clock: the latest time it has been given.
    #[must_use]
    pub const fn now(&self) -> Timestamp {
        self.now
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
    /// expected kind. Anything else is stale and ignored.
    fn complete(&mut self, effect: EffectId, is_model: bool) -> bool {
        let Some(open) = self.state.open_effects().get(&effect) else {
            return false;
        };
        if matches!(open.effect, Effect::CallModel { .. }) != is_model
            || !self.dispatched.remove(&effect)
        {
            return false;
        }
        self.cancelling.remove(&effect);
        true
    }

    fn stop_reason(&self, effect: EffectId) -> StopReason {
        let cancelled = self
            .state
            .open_effects()
            .get(&effect)
            .and_then(|open| self.state.cancelled_at(open.turn));
        if cancelled.is_some() {
            StopReason::TurnCancelled
        } else {
            StopReason::TimedOut
        }
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

    /// Records an outcome the session decided without the effect, and drops
    /// the effect if it is still running.
    fn resolve(&mut self, effect: EffectId, body: RecordBody) {
        self.produce(body);
        if self.dispatched.remove(&effect) {
            self.cancelling.remove(&effect);
            self.outbox.push_back(Submission::Abandon { effect });
        }
    }

    /// Produces every record due at the session's clock, then hands the
    /// shell what is now safe to do.
    fn drive(&mut self) {
        // Asking a tool to stop starts its grace period, which may already
        // be over (a grace of zero), so produce again until nothing changes.
        loop {
            self.produce_due();
            if !self.cancel_overdue_tools() {
                break;
            }
        }
        self.dispatch_durable_intents();
        self.flush();
    }

    /// When an unsafe tool that has been asked to stop is abandoned: its
    /// grace period after the request.
    fn abandon_at(&self, id: EffectId, open: &OpenEffect) -> Option<Timestamp> {
        let Kind::Tool {
            safety: Safety::Unsafe,
            grace,
            ..
        } = open.kind
        else {
            return None;
        };
        self.cancelling.get(&id).map(|asked| asked.after(grace))
    }

    fn produce_due(&mut self) {
        loop {
            if let Some((effect, body)) = self.next_resolution() {
                self.resolve(effect, body);
            } else if let Some(body) = self.state.next_record(self.now) {
                self.produce(body);
            } else {
                return;
            }
        }
    }

    fn next_resolution(&self) -> Option<(EffectId, RecordBody)> {
        self.state
            .open_effects()
            .iter()
            .find_map(|(id, open)| self.decide(*id, open).map(|body| (*id, body)))
    }

    /// The outcome the session must record for `open` now, without waiting
    /// for the effect, if any. Claims only what is known: an effect that may
    /// have started is never reported as not run, and an unsafe tool that
    /// has not confirmed it stopped is reported as unknown.
    fn decide(&self, id: EffectId, open: &OpenEffect) -> Option<RecordBody> {
        let cancelled = self.state.cancelled_at(open.turn).is_some();
        let started = self.dispatched.contains(&id) || id.intent() < self.recovered;
        let timers = open.timers();
        let passed = |deadline: Option<Timestamp>| deadline.is_some_and(|due| self.now >= due);
        let tool = |outcome| {
            Some(RecordBody::ToolResolved {
                effect: id,
                outcome,
            })
        };
        let reason = if cancelled {
            Some(StopReason::TurnCancelled)
        } else if passed(timers.soft.or(timers.hard)) {
            Some(StopReason::TimedOut)
        } else {
            None
        };
        match open.kind {
            Kind::UnknownTool => tool(ToolOutcome::UnknownTool),
            Kind::Model { .. } => {
                let outcome = if cancelled {
                    ModelOutcome::Cancelled
                } else if passed(timers.hard) {
                    ModelOutcome::Failed {
                        reason: String::from("timed out"),
                        retryable: true,
                        retry_after: None,
                    }
                } else {
                    return None;
                };
                Some(RecordBody::ModelResolved {
                    effect: id,
                    outcome,
                })
            }
            // A tool that may have started is never claimed to have
            // stopped: an idempotent one is dropped at once, an unsafe one
            // once it has had its grace period to confirm; either way, what
            // it did is unknown.
            Kind::Tool { safety, .. } => match (safety, started, reason) {
                (_, false, Some(reason)) => tool(ToolOutcome::NotRun { reason }),
                (Safety::Idempotent, true, Some(_)) => tool(ToolOutcome::Unknown {
                    cause: UnknownCause::Abandoned,
                }),
                (Safety::Unsafe, true, Some(_)) if passed(self.abandon_at(id, open)) => {
                    tool(ToolOutcome::Unknown {
                        cause: UnknownCause::Abandoned,
                    })
                }
                (Safety::Unsafe, true, Some(_)) | (_, _, None) => None,
            },
        }
    }

    fn dispatch_durable_intents(&mut self) {
        let ready: Vec<Effect> = self
            .state
            .open_effects()
            .iter()
            .filter(|(id, open)| {
                // Write-ahead: the intent is durable, and so is this life's
                // fence. The planted bug skips both.
                let safe = id.intent() < self.durable && self.durable > self.recovered;
                (safe || cfg!(piton_planted_bug))
                    && !self.dispatched.contains(id)
                    && self.state.cancelled_at(open.turn).is_none()
                    && open.kind != Kind::UnknownTool
            })
            .map(|(_, open)| open.effect.clone())
            .collect();
        for effect in ready {
            self.dispatched.insert(effect.id());
            self.outbox.push_back(Submission::Dispatch { effect });
        }
    }

    /// Asks running unsafe tools to stop once their turn is cancelled or
    /// their timeout passes; their grace period runs from then. Returns
    /// whether it asked any.
    fn cancel_overdue_tools(&mut self) -> bool {
        let overdue: Vec<EffectId> = self
            .dispatched
            .iter()
            .filter(|id| !self.cancelling.contains_key(id))
            .copied()
            .filter(|id| {
                self.state.open_effects().get(id).is_some_and(|open| {
                    let overdue = self.state.cancelled_at(open.turn).is_some()
                        || open.timers().soft.is_some_and(|due| self.now >= due);
                    overdue
                        && matches!(
                            open.kind,
                            Kind::Tool {
                                safety: Safety::Unsafe,
                                ..
                            }
                        )
                })
            })
            .collect();
        let asked = !overdue.is_empty();
        for effect in overdue {
            self.cancelling.insert(effect, self.now);
            self.outbox.push_back(Submission::Cancel { effect });
        }
        asked
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
