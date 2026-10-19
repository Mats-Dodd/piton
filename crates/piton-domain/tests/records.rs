//! Records are forever: every record a released version wrote must keep
//! deserializing to the same value and serializing to the same bytes. Never
//! edit a fixture; a new schema version gets a new one.

use piton_domain::{
    ContentBlock, EffectId, Json, Message, Millis, ModelOutcome, ModelPolicy, ModelRequest,
    ModelResponse, Offset, Record, RecordBody, RequestId, RetryPolicy, Role, SCHEMA_VERSION,
    Safety, StopReason, Timestamp, ToolCall, ToolName, ToolOutcome, ToolSpec, ToolUseId, TurnEnd,
    TurnId, UnknownCause,
};

const SCHEMA_1: &str = include_str!("fixtures/schema-1.ndjson");

fn tool_call() -> ToolCall {
    ToolCall {
        tool_use_id: ToolUseId::new("use-1".to_owned()),
        name: ToolName::new("search".to_owned()),
        input: Json::parse(r#"{"query": "piton", "limit": 3}"#).expect("valid JSON"),
    }
}

/// One record of every kind, and every outcome variant.
fn schema_1_records() -> Vec<Record> {
    let turn = TurnId::new(Offset::new(3));
    let model_call = EffectId::new(Offset::new(4));
    let tool = EffectId::new(Offset::new(6));
    let bodies = [
        RecordBody::SessionCreated {
            schema: SCHEMA_VERSION,
            tools: Vec::from([
                ToolSpec {
                    name: ToolName::new("search".to_owned()),
                    safety: Safety::Idempotent,
                    timeout: Millis::new(30_000),
                    cancel_grace: Millis::ZERO,
                },
                ToolSpec {
                    name: ToolName::new("send_email".to_owned()),
                    safety: Safety::Unsafe,
                    timeout: Millis::new(60_000),
                    cancel_grace: Millis::new(5_000),
                },
            ]),
            model: ModelPolicy {
                timeout: Millis::new(120_000),
                retry: RetryPolicy {
                    max_attempts: 3,
                    initial_backoff: Millis::new(1_000),
                    max_backoff: Millis::new(30_000),
                },
            },
        },
        RecordBody::SessionResumed,
        RecordBody::InputAdmitted {
            request_id: RequestId::new("request-1".to_owned()),
            text: "find piton".to_owned(),
        },
        RecordBody::TurnStarted {
            request_id: RequestId::new("request-1".to_owned()),
        },
        RecordBody::ModelIntent {
            turn,
            request: ModelRequest {
                messages: Vec::from([Message {
                    role: Role::User,
                    content: Vec::from([ContentBlock::Text {
                        text: "find piton".to_owned(),
                    }]),
                }]),
            },
        },
        RecordBody::ModelResolved {
            effect: model_call,
            outcome: ModelOutcome::Responded {
                response: ModelResponse {
                    content: Vec::from([ContentBlock::ToolUse {
                        id: ToolUseId::new("use-1".to_owned()),
                        name: ToolName::new("search".to_owned()),
                        input: tool_call().input,
                    }]),
                },
            },
        },
        RecordBody::ToolIntent {
            turn,
            call: tool_call(),
            safety: Safety::Unsafe,
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::Returned {
                output: Json::string("found"),
                is_error: false,
            },
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::NotRun {
                reason: StopReason::TurnCancelled,
            },
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::Stopped {
                reason: StopReason::TimedOut,
            },
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::Unknown {
                cause: UnknownCause::Restart,
            },
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::Unknown {
                cause: UnknownCause::Abandoned,
            },
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::UnknownTool,
        },
        RecordBody::ModelResolved {
            effect: model_call,
            outcome: ModelOutcome::Failed {
                reason: "overloaded".to_owned(),
                retryable: true,
                retry_after: Some(Millis::new(2_000)),
            },
        },
        RecordBody::ModelResolved {
            effect: model_call,
            outcome: ModelOutcome::Failed {
                reason: "invalid request".to_owned(),
                retryable: false,
                retry_after: None,
            },
        },
        RecordBody::ModelResolved {
            effect: model_call,
            outcome: ModelOutcome::Cancelled,
        },
        RecordBody::TurnCancelled { turn },
        RecordBody::TurnEnded {
            turn,
            reason: TurnEnd::Completed,
        },
        RecordBody::TurnEnded {
            turn,
            reason: TurnEnd::Failed,
        },
        RecordBody::TurnEnded {
            turn,
            reason: TurnEnd::Cancelled,
        },
    ];
    bodies
        .into_iter()
        .zip(0..)
        .map(|(body, offset)| Record {
            offset: Offset::new(offset),
            at: Timestamp::from_millis(1_700_000_000_000),
            body,
        })
        .collect()
}

fn to_ndjson(records: &[Record]) -> String {
    records
        .iter()
        .map(|record| serde_json::to_string(record).expect("a record serializes") + "\n")
        .collect()
}

#[test]
fn schema_1_records_still_deserialize() {
    let parsed: Vec<Record> = SCHEMA_1
        .lines()
        .map(|line| serde_json::from_str(line).expect("a schema 1 record parses"))
        .collect();
    assert_eq!(
        parsed,
        schema_1_records(),
        "schema 1 records changed meaning"
    );
}

#[test]
fn schema_1_records_still_serialize_identically() {
    assert_eq!(
        to_ndjson(&schema_1_records()),
        SCHEMA_1,
        "schema 1 wire format changed"
    );
}

#[test]
fn json_is_canonical() {
    let spaced = Json::parse(r#"{ "b": 1, "a": [true, null] }"#).expect("valid JSON");
    let compact = Json::parse(r#"{"a":[true,null],"b":1}"#).expect("valid JSON");
    assert_eq!(spaced, compact, "equal JSON values compare equal");
    assert_eq!(
        spaced.as_str(),
        r#"{"a":[true,null],"b":1}"#,
        "keys sorted, no whitespace"
    );
    assert!(Json::parse("{").is_err(), "invalid JSON is rejected");
}

/// Writes the fixture. Run once when a new schema version is introduced,
/// never to paper over a change to an existing one.
#[test]
#[ignore = "writes tests/fixtures/schema-1.ndjson; only for a new schema version"]
fn write_schema_1_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/schema-1.ndjson"
    );
    std::fs::write(path, to_ndjson(&schema_1_records())).expect("the fixture is writable");
}

#[test]
fn backoff_doubles_up_to_its_cap() {
    let retry = RetryPolicy {
        max_attempts: 6,
        initial_backoff: Millis::new(100),
        max_backoff: Millis::new(500),
    };
    let waits: Vec<u64> = (1..=5)
        .map(|failures| retry.backoff(failures).get())
        .collect();
    assert_eq!(waits, [100, 200, 400, 500, 500], "doubling, then capped");
}
