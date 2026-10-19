use serde::{Deserialize, Serialize};

use crate::{InvalidPolicy, Json, Millis, ToolName, ToolUseId};

/// Whether running a tool twice, or only partly, matters. Decides both what
/// recovery does with a tool that was in flight at a restart and what
/// cancellation does with one that is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// Harmless to repeat or abandon, or deduplicated on the effect id:
    /// recovery dispatches it again under the same id, and cancellation simply
    /// drops it, recording its outcome as unknown if it may have started.
    Idempotent,
    /// Must not run twice: recovery reports it as possibly run, and
    /// cancellation asks it to stop, waiting up to its grace period for an
    /// answer.
    Unsafe,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: ToolName,
    pub safety: Safety,
    /// How long the tool may run, from its intent, before it is stopped.
    pub timeout: Millis,
    /// How long an unsafe tool has to confirm it stopped before the session
    /// abandons it.
    pub cancel_grace: Millis,
}

impl ToolSpec {
    /// Checks the tool can be run under the session's rules.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPolicy::ZeroToolTimeout`] for a zero timeout.
    pub fn validate(&self) -> Result<(), InvalidPolicy> {
        if self.timeout == Millis::ZERO {
            Err(InvalidPolicy::ZeroToolTimeout)
        } else {
            Ok(())
        }
    }
}

/// A tool use the model asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_use_id: ToolUseId,
    pub name: ToolName,
    pub input: Json,
}

/// Why a tool stopped, or never started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    TurnCancelled,
    TimedOut,
}

/// Why a tool's outcome cannot be known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownCause {
    /// The session restarted while the tool may have been running.
    Restart,
    /// The session stopped waiting for the tool: an idempotent tool when it
    /// was cancelled or timed out, an unsafe one when it did not confirm it
    /// stopped within its grace period.
    Abandoned,
}

/// How a tool call ended. Every outcome claims only what is known: a tool is
/// reported as stopped or not run only when that is certain.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutcome {
    /// The tool ran and produced output; `is_error` is the tool's own verdict.
    Returned { output: Json, is_error: bool },
    /// The tool was never started.
    NotRun { reason: StopReason },
    /// The tool is known to have stopped before producing output.
    Stopped { reason: StopReason },
    /// The tool may or may not have run, in whole or in part.
    Unknown { cause: UnknownCause },
    /// The model named a tool the session does not have.
    UnknownTool,
}

impl ToolOutcome {
    /// How the outcome reads to the model: `(content, is_error)`.
    #[must_use]
    pub fn as_tool_result(&self) -> (Json, bool) {
        let text = match self {
            Self::Returned { output, is_error } => return (output.clone(), *is_error),
            Self::NotRun {
                reason: StopReason::TurnCancelled,
            } => "not run: the turn was cancelled",
            Self::NotRun {
                reason: StopReason::TimedOut,
            } => "not run: timed out before starting",
            Self::Stopped {
                reason: StopReason::TurnCancelled,
            } => "stopped before completing: the turn was cancelled",
            Self::Stopped {
                reason: StopReason::TimedOut,
            } => "stopped before completing: timed out",
            Self::Unknown {
                cause: UnknownCause::Restart,
            } => "interrupted by a restart: it may or may not have run, in whole or in part",
            Self::Unknown {
                cause: UnknownCause::Abandoned,
            } => "abandoned before it finished: it may or may not have run, in whole or in part",
            Self::UnknownTool => "no tool with this name exists",
        };
        (Json::string(text), true)
    }
}
