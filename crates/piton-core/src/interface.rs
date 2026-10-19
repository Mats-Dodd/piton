use alloc::{string::String, vec::Vec};

use piton_domain::{
    EffectId, Json, Millis, ModelRequest, ModelResponse, Offset, Record, RequestId, Timestamp,
    ToolCall, TurnId,
};
use serde::{Deserialize, Serialize};

/// Everything the shell tells a session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub enum Input {
    /// Storage durably holds the outstanding append; `end` is the offset just
    /// past its last record.
    Appended {
        end: Offset,
    },
    ModelResponded {
        effect: EffectId,
        response: ModelResponse,
    },
    ModelFailed {
        effect: EffectId,
        reason: String,
        retryable: bool,
        retry_after: Option<Millis>,
    },
    ToolReturned {
        effect: EffectId,
        output: Json,
        is_error: bool,
    },
    /// The tool stopped because the session asked it to.
    EffectCancelled {
        effect: EffectId,
    },
    Submit {
        request_id: RequestId,
        text: String,
    },
    CancelTurn {
        turn: TurnId,
    },
    /// Nothing new happened, but time has passed. The shell sends it once
    /// [`Session::deadline`](crate::Session::deadline) is reached; an early
    /// tick does nothing.
    Tick,
}

/// Work the session hands the shell.
///
/// Every `Dispatch` is answered by exactly one completion input for its
/// effect, unless the session abandons the effect first.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "submission", rename_all = "snake_case")]
pub enum Submission {
    /// Append `records` atomically, provided the log currently ends at
    /// `expected`.
    Append {
        expected: Offset,
        records: Vec<Record>,
    },
    Dispatch {
        effect: Effect,
    },
    /// Ask a running tool to stop. It answers with `EffectCancelled` if it
    /// stops, or with its result if it finishes first.
    Cancel {
        effect: EffectId,
    },
    /// Stop running the effect and drop it. Its outcome is already recorded;
    /// any completion that still arrives is ignored.
    Abandon {
        effect: EffectId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    CallModel { id: EffectId, request: ModelRequest },
    RunTool { id: EffectId, call: ToolCall },
}

impl Effect {
    #[must_use]
    pub const fn id(&self) -> EffectId {
        match self {
            Self::CallModel { id, .. } | Self::RunTool { id, .. } => *id,
        }
    }
}

/// One thing that crossed the session's boundary, as a shell records it.
///
/// A session's entries, in order, replay exactly: they hold every input,
/// every timestamp and every submission. The journal is for observing and
/// replaying a session, never for recovering one; the log alone is the truth.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum JournalEntry {
    /// The session was built from a durable log of `log_len` records.
    Recovered {
        now: Timestamp,
        log_len: Offset,
        submissions: Vec<Submission>,
    },
    /// The session handled `input`.
    Handled {
        now: Timestamp,
        input: Input,
        submissions: Vec<Submission>,
    },
}
