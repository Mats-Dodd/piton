use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Serialize};

use crate::{
    EffectId, ModelOutcome, ModelPolicy, ModelRequest, Offset, RequestId, Safety, Timestamp,
    ToolCall, ToolOutcome, ToolSpec, TurnId,
};

/// The log format version written into `SessionCreated`.
pub const SCHEMA_VERSION: u32 = 1;

/// One entry in a session's durable log.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Record {
    pub offset: Offset,
    pub at: Timestamp,
    pub body: RecordBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordBody {
    /// Always the first record.
    SessionCreated {
        schema: u32,
        tools: Vec<ToolSpec>,
        model: ModelPolicy,
    },
    /// A new life of the session began from the log before this record. Once
    /// it is durable, no append from an earlier life can land, so the session
    /// may act on what it recovered.
    SessionResumed,
    /// A client submission, admitted at most once per request id. Queued until
    /// a turn picks it up.
    InputAdmitted {
        request_id: RequestId,
        text: String,
    },
    /// Starts a turn for a queued input. The turn's id is this record's offset.
    TurnStarted {
        request_id: RequestId,
    },
    /// Intent to call the model. The effect's id is this record's offset.
    ModelIntent {
        turn: TurnId,
        request: ModelRequest,
    },
    ModelResolved {
        effect: EffectId,
        outcome: ModelOutcome,
    },
    /// Intent to run a tool. The effect's id is this record's offset. A tool
    /// the session does not have is recorded as unsafe.
    ToolIntent {
        turn: TurnId,
        call: ToolCall,
        safety: Safety,
    },
    ToolResolved {
        effect: EffectId,
        outcome: ToolOutcome,
    },
    TurnCancelled {
        turn: TurnId,
    },
    TurnEnded {
        turn: TurnId,
        reason: TurnEnd,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEnd {
    /// The model replied without asking for tools.
    Completed,
    /// A model call failed for good: it was not retryable, returned
    /// something unusable, or ran out of attempts.
    Failed,
    Cancelled,
}
