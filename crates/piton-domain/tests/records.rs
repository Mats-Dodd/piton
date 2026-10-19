//! Records are forever: every record a released version wrote must keep
//! deserializing to the same value and serializing to the same bytes. Never
//! edit a fixture; a new schema version gets a new one.

use piton_domain::{
    ContentBlock, EffectId, Json, Message, ModelOutcome, ModelRequest, ModelResponse, Offset,
    Record, RecordBody, ReplayPolicy, RequestId, Role, SCHEMA_VERSION, Timestamp, ToolCall,
    ToolName, ToolOutcome, ToolSpec, ToolUseId, TurnEnd, TurnId,
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
    let turn = TurnId::new(Offset::new(2));
    let model_call = EffectId::new(Offset::new(3));
    let tool = EffectId::new(Offset::new(5));
    let bodies = [
        RecordBody::SessionCreated {
            schema: SCHEMA_VERSION,
            tools: Vec::from([ToolSpec {
                name: ToolName::new("search".to_owned()),
                replay: ReplayPolicy::Redispatch,
            }]),
        },
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
            replay: ReplayPolicy::Interrupt,
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
            outcome: ToolOutcome::Cancelled,
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::Interrupted,
        },
        RecordBody::ToolResolved {
            effect: tool,
            outcome: ToolOutcome::UnknownTool,
        },
        RecordBody::ModelResolved {
            effect: model_call,
            outcome: ModelOutcome::Failed {
                reason: "overloaded".to_owned(),
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
