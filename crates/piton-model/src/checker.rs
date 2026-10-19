use piton_domain::{
    EffectId, ModelOutcome, Record, RecordBody, StopReason, Timestamp, ToolOutcome, TurnEnd,
    UnknownCause,
};
use stateright::{Model, Property};

use crate::{EnvAction, World, WorldConfig, check, check_exploration, enabled, step};

/// Exhaustive exploration of the real session with Stateright.
#[derive(Clone, Debug)]
pub struct Checked(pub WorldConfig);

impl Model for Checked {
    type State = World;
    type Action = EnvAction;

    fn init_states(&self) -> Vec<World> {
        Vec::from([World::new(&self.0, Timestamp::EPOCH)])
    }

    fn actions(&self, state: &World, actions: &mut Vec<EnvAction>) {
        actions.extend(enabled(&self.0, state));
    }

    fn next_state(&self, last_state: &World, action: EnvAction) -> Option<World> {
        let mut next = last_state.clone();
        step(&mut next, action);
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        Vec::from([
            Property::always("invariants hold", |model: &Self, world: &World| {
                check(world).is_ok() && check_exploration(&model.0, world).is_ok()
            }),
            // The `sometimes` properties guard against vacuous passes: each
            // names a hard path the bounds must still reach.
            Property::sometimes(
                "a turn completes after running a tool",
                |_, world: &World| {
                    durable_has(world, |body| {
                        matches!(
                            body,
                            RecordBody::ToolResolved {
                                outcome: ToolOutcome::Returned { .. },
                                ..
                            }
                        )
                    }) && durable_has(world, |body| {
                        matches!(
                            body,
                            RecordBody::TurnEnded {
                                reason: TurnEnd::Completed,
                                ..
                            }
                        )
                    })
                },
            ),
            Property::sometimes(
                "a restart leaves an unsafe tool unknown",
                |_, world: &World| {
                    durable_has(world, |body| {
                        matches!(
                            body,
                            RecordBody::ToolResolved {
                                outcome: ToolOutcome::Unknown {
                                    cause: UnknownCause::Restart
                                },
                                ..
                            }
                        )
                    })
                },
            ),
            Property::sometimes("an unsafe tool confirms it stopped", |_, world: &World| {
                world.env().any_stopped()
                    && durable_has(world, |body| {
                        matches!(
                            body,
                            RecordBody::ToolResolved {
                                outcome: ToolOutcome::Stopped { .. },
                                ..
                            }
                        )
                    })
            }),
            Property::sometimes("an unsafe tool is abandoned", |_, world: &World| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ToolResolved {
                            outcome: ToolOutcome::Unknown {
                                cause: UnknownCause::Abandoned
                            },
                            ..
                        }
                    )
                })
            }),
            Property::sometimes("a running tool times out", |_, world: &World| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ToolResolved {
                            outcome: ToolOutcome::Stopped {
                                reason: StopReason::TimedOut
                            },
                            ..
                        }
                    )
                })
            }),
            Property::sometimes(
                "a cancel stops a tool before it runs",
                |_, world: &World| {
                    durable_has(world, |body| {
                        matches!(
                            body,
                            RecordBody::ToolResolved {
                                outcome: ToolOutcome::NotRun {
                                    reason: StopReason::TurnCancelled
                                },
                                ..
                            }
                        )
                    })
                },
            ),
            Property::sometimes("an unknown tool is answered", |_, world: &World| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ToolResolved {
                            outcome: ToolOutcome::UnknownTool,
                            ..
                        }
                    )
                })
            }),
            Property::sometimes("a failed model call is retried", |_, world: &World| {
                retried(world)
            }),
            Property::sometimes("a model call fails for good", |_, world: &World| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::TurnEnded {
                            reason: TurnEnd::Failed,
                            ..
                        }
                    )
                })
            }),
            Property::sometimes("a late append lands", |_, world: &World| {
                world.env().late_landed()
            }),
        ])
    }
}

fn durable_has(world: &World, predicate: impl Fn(&RecordBody) -> bool) -> bool {
    world
        .env()
        .durable()
        .iter()
        .map(|record: &Record| &record.body)
        .any(predicate)
}

/// Whether some model call followed a retryable failure in the same turn.
fn retried(world: &World) -> bool {
    let mut failed_turns = Vec::new();
    let mut intents = Vec::new();
    for record in world.env().durable() {
        match &record.body {
            RecordBody::ModelIntent { turn, .. } => {
                if failed_turns.contains(turn) {
                    return true;
                }
                intents.push((EffectId::new(record.offset), *turn));
            }
            RecordBody::ModelResolved {
                effect,
                outcome:
                    ModelOutcome::Failed {
                        retryable: true, ..
                    },
            } => {
                if let Some((_, turn)) = intents.iter().find(|(id, _)| id == effect) {
                    failed_turns.push(*turn);
                }
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::SessionResumed
            | RecordBody::InputAdmitted { .. }
            | RecordBody::TurnStarted { .. }
            | RecordBody::ModelResolved { .. }
            | RecordBody::ToolIntent { .. }
            | RecordBody::ToolResolved { .. }
            | RecordBody::TurnCancelled { .. }
            | RecordBody::TurnEnded { .. } => {}
        }
    }
    false
}
