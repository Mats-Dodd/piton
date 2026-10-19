use alloc::{string::String, vec::Vec};

use piton_domain::{
    EffectId, Json, ModelRequest, ModelResponse, Offset, Record, RequestId, ToolCall, TurnId,
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
    },
    ToolReturned {
        effect: EffectId,
        output: Json,
        is_error: bool,
    },
    /// The effect stopped because the session asked to cancel it.
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
}

/// Work the session hands the shell. Every `Dispatch` must eventually be
/// answered by exactly one completion input for its effect.
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
    /// Best effort: the effect still completes exactly once, either as
    /// cancelled or with whatever it produced first.
    Cancel {
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
