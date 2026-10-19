use piton_domain::{ModelOutcome, Record, RecordBody, ToolOutcome, TurnEnd};
use stateright::{Model, Property};

use crate::{EnvAction, World, WorldConfig, check, enabled, step};

/// Exhaustive exploration of the real session with Stateright.
#[derive(Clone, Debug)]
pub struct Checked(pub WorldConfig);

impl Model for Checked {
    type State = World;
    type Action = EnvAction;

    fn init_states(&self) -> Vec<World> {
        Vec::from([World::new(&self.0)])
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
                check(&model.0, world).is_ok()
            }),
            // The `sometimes` properties guard against vacuous passes: each
            // names a hard path the bounds must still reach.
            Property::sometimes("a turn completes after running a tool", |_, world| {
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
            }),
            Property::sometimes("a restart interrupts an unsafe tool", |_, world| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ToolResolved {
                            outcome: ToolOutcome::Interrupted,
                            ..
                        }
                    )
                })
            }),
            Property::sometimes("a cancel stops a dispatched tool", |_, world| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ToolResolved {
                            outcome: ToolOutcome::Cancelled,
                            ..
                        }
                    )
                })
            }),
            Property::sometimes("an unknown tool is answered", |_, world| {
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
            Property::sometimes("an invalid reply fails the turn", |_, world| {
                durable_has(world, |body| {
                    matches!(
                        body,
                        RecordBody::ModelResolved {
                            outcome: ModelOutcome::Failed { .. },
                            ..
                        }
                    )
                })
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
