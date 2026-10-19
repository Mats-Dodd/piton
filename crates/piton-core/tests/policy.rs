//! A session's policy is checked before it is trusted: a zero timeout would
//! let one input retry a model call any number of times without time
//! passing.

use piton_core::{RecoveryError, Session};
use piton_domain::{
    Millis, ModelPolicy, Offset, Record, RecordBody, RetryPolicy, SCHEMA_VERSION, Safety,
    Timestamp, ToolName, ToolSpec,
};

const SOUND: ModelPolicy = ModelPolicy {
    timeout: Millis::new(1_000),
    retry: RetryPolicy {
        max_attempts: 3,
        initial_backoff: Millis::new(10),
        max_backoff: Millis::new(100),
    },
};

fn log(tools: Vec<ToolSpec>, model: ModelPolicy) -> Vec<Record> {
    Vec::from([Record {
        offset: Offset::ZERO,
        at: Timestamp::EPOCH,
        body: RecordBody::SessionCreated {
            schema: SCHEMA_VERSION,
            tools,
            model,
        },
    }])
}

fn tool(timeout: u64) -> ToolSpec {
    ToolSpec {
        name: ToolName::new("search".to_owned()),
        safety: Safety::Idempotent,
        timeout: Millis::new(timeout),
        cancel_grace: Millis::ZERO,
    }
}

#[test]
fn a_sound_policy_recovers() {
    assert!(
        Session::from_log(Timestamp::EPOCH, &log(Vec::from([tool(10)]), SOUND)).is_ok(),
        "a sound policy is accepted"
    );
}

#[test]
fn an_unsound_policy_is_refused() {
    let unsound = [
        (
            Vec::new(),
            ModelPolicy {
                timeout: Millis::ZERO,
                ..SOUND
            },
        ),
        (Vec::from([tool(0)]), SOUND),
        (
            Vec::new(),
            ModelPolicy {
                retry: RetryPolicy {
                    max_attempts: 0,
                    ..SOUND.retry
                },
                ..SOUND
            },
        ),
        (
            Vec::new(),
            ModelPolicy {
                retry: RetryPolicy {
                    max_backoff: Millis::new(1),
                    ..SOUND.retry
                },
                ..SOUND
            },
        ),
    ];
    for (tools, model) in unsound {
        let recovered = Session::from_log(Timestamp::EPOCH, &log(tools.clone(), model)).err();
        assert!(
            matches!(recovered, Some(RecoveryError::InvalidPolicy(_))),
            "{tools:?} {model:?} is refused: {recovered:?}"
        );
    }
}
