use serde::{Deserialize, Serialize};

use crate::{Json, ToolName, ToolUseId};

/// What recovery does with a tool call that was dispatched but has no recorded
/// outcome when the session restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayPolicy {
    /// Dispatch it again under the same effect id. For tools that are
    /// harmless to repeat or that deduplicate on the effect id.
    Redispatch,
    /// Never run it again; tell the model it was interrupted and may have
    /// partially run.
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub replay: ReplayPolicy,
}

/// A tool use the model asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_use_id: ToolUseId,
    pub name: ToolName,
    pub input: Json,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutcome {
    /// The tool ran and produced output; `is_error` is the tool's own verdict.
    Returned { output: Json, is_error: bool },
    /// The turn was cancelled before the tool produced output.
    Cancelled,
    /// The session restarted while the tool was in flight and its policy
    /// forbids running it again.
    Interrupted,
    /// The model named a tool the session does not have.
    UnknownTool,
}

impl ToolOutcome {
    /// How the outcome reads to the model: `(content, is_error)`.
    #[must_use]
    pub fn as_tool_result(&self) -> (Json, bool) {
        match self {
            Self::Returned { output, is_error } => (output.clone(), *is_error),
            Self::Cancelled => (Json::string("cancelled before completing"), true),
            Self::Interrupted => (
                Json::string("interrupted by a restart; it may have partially run"),
                true,
            ),
            Self::UnknownTool => (Json::string("no tool with this name exists"), true),
        }
    }
}
