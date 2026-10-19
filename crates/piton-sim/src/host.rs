//! The program on the simulated host: a runtime and a client using it.
//! Turmoil runs it afresh after every crash, so everything it must remember
//! lives in the [`SimWorld`].

use std::{sync::Arc, time::Duration};

use piton_domain::{Offset, RecordBody, RequestId};
use piton_runtime::{Follower, Ports, Runtime, SessionHandle};

use crate::{
    MODEL_POLICY,
    ports::{Journal, Model, SimClock, SkewedClock, Store, Tools},
    tools,
    world::SimWorld,
};

/// Runs until the host crashes. Never fails: the client copes with
/// whatever happens by reopening the session.
#[expect(
    clippy::infinite_loop,
    reason = "turmoil requires a host to return a Result; this one only ever stops by crashing"
)]
pub(crate) async fn run(world: Arc<SimWorld>) -> turmoil::Result {
    let runtime = Runtime::new(Ports {
        store: Arc::new(Store(Arc::clone(&world))),
        model: Arc::new(Model(Arc::clone(&world))),
        tools: Arc::new(Tools(Arc::clone(&world))),
        clock: Arc::new(SkewedClock(Arc::clone(&world))),
        journal: Arc::new(Journal(Arc::clone(&world))),
    });
    loop {
        if let Some(session) = open(&runtime, &world).await {
            let follower = tokio::spawn(watch(session.follow(Offset::ZERO), Arc::clone(&world)));
            drive(&session, &world).await;
            follower.abort();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Creates the session the first time, opens it after that.
async fn open(runtime: &Runtime, world: &SimWorld) -> Option<SessionHandle> {
    let opened = if world.store.records(&world.session).is_empty() {
        runtime
            .create(world.session.clone(), tools(), MODEL_POLICY)
            .await
    } else {
        runtime.open(world.session.clone()).await
    };
    opened.ok()
}

/// Submits every planned request until it is seen admitted, cancelling the
/// odd turn, until the session stops.
async fn drive(session: &SessionHandle, world: &SimWorld) {
    loop {
        let now = SimClock::millis();
        for index in 0..world.plan.requests {
            let request_id = RequestId::new(format!("request-{index}"));
            let due = u64::from(index).saturating_mul(world.plan.request_gap_millis);
            if now >= due && !world.has_seen_admitted(&request_id) {
                let text = format!("task {index}");
                if session.submit(request_id, text).await.is_err() {
                    return;
                }
            }
        }
        if let Some(turn) = world.active_turn()
            && world.fault(60)
            && session.cancel(turn).await.is_err()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(world.pick(5, 30))).await;
    }
}

/// Tells the client what the durable log says.
async fn watch(mut follower: Follower, world: Arc<SimWorld>) {
    while let Ok(record) = follower.next().await {
        match record.body {
            RecordBody::InputAdmitted { request_id, .. } => world.saw_admitted(request_id),
            RecordBody::TurnStarted { .. } => {
                world.saw_turn(Some(piton_domain::TurnId::new(record.offset)));
            }
            RecordBody::TurnEnded { .. } => world.saw_turn(None),
            RecordBody::SessionCreated { .. }
            | RecordBody::SessionResumed
            | RecordBody::ModelIntent { .. }
            | RecordBody::ModelResolved { .. }
            | RecordBody::ToolIntent { .. }
            | RecordBody::ToolResolved { .. }
            | RecordBody::TurnCancelled { .. } => {}
        }
    }
}
