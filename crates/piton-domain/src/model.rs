use alloc::{collections::BTreeSet, string::String, vec::Vec};
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{ContentBlock, Message, Millis, Role, ToolCall, ToolUseId};

/// Everything a model call needs, fully determined by the log.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelRequest {
    pub messages: Vec<Message>,
}

impl ModelRequest {
    /// Checks the shape providers require: the conversation starts and ends
    /// with the user, roles alternate, and every tool use is answered by
    /// exactly one result in the very next message.
    ///
    /// # Errors
    ///
    /// Returns the first rule the request breaks.
    pub fn validate(&self) -> Result<(), MalformedRequest> {
        let first = self.messages.first().ok_or(MalformedRequest::Empty)?;
        if first.role != Role::User {
            return Err(MalformedRequest::StartsWithAssistant);
        }
        let last = self.messages.last().ok_or(MalformedRequest::Empty)?;
        if last.role != Role::User {
            return Err(MalformedRequest::EndsWithAssistant);
        }
        let mut unanswered = BTreeSet::new();
        let mut previous_role = None;
        for message in &self.messages {
            if previous_role == Some(message.role) {
                return Err(MalformedRequest::RolesDoNotAlternate);
            }
            previous_role = Some(message.role);
            match message.role {
                Role::Assistant => {
                    for block in &message.content {
                        match block {
                            ContentBlock::ToolUse { id, .. } => {
                                if !unanswered.insert(id.clone()) {
                                    return Err(MalformedRequest::DuplicateToolUse(id.clone()));
                                }
                            }
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                return Err(MalformedRequest::ResultFromAssistant(
                                    tool_use_id.clone(),
                                ));
                            }
                            ContentBlock::Text { .. } => {}
                        }
                    }
                }
                Role::User => {
                    for block in &message.content {
                        match block {
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                if !unanswered.remove(tool_use_id) {
                                    return Err(MalformedRequest::UnmatchedResult(
                                        tool_use_id.clone(),
                                    ));
                                }
                            }
                            ContentBlock::ToolUse { id, .. } => {
                                return Err(MalformedRequest::ToolUseFromUser(id.clone()));
                            }
                            ContentBlock::Text { .. } => {}
                        }
                    }
                    if let Some(id) = unanswered.pop_first() {
                        return Err(MalformedRequest::UnansweredToolUse(id));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MalformedRequest {
    Empty,
    StartsWithAssistant,
    EndsWithAssistant,
    RolesDoNotAlternate,
    DuplicateToolUse(ToolUseId),
    ResultFromAssistant(ToolUseId),
    ToolUseFromUser(ToolUseId),
    UnmatchedResult(ToolUseId),
    UnansweredToolUse(ToolUseId),
}

impl fmt::Display for MalformedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("request has no messages"),
            Self::StartsWithAssistant => formatter.write_str("first message is not from the user"),
            Self::EndsWithAssistant => formatter.write_str("last message is not from the user"),
            Self::RolesDoNotAlternate => {
                formatter.write_str("two consecutive messages share a role")
            }
            Self::DuplicateToolUse(id) => {
                write!(formatter, "tool use {} appears twice", id.as_str())
            }
            Self::ResultFromAssistant(id) => {
                write!(formatter, "assistant sent the result for {}", id.as_str())
            }
            Self::ToolUseFromUser(id) => write!(formatter, "user sent tool use {}", id.as_str()),
            Self::UnmatchedResult(id) => {
                write!(
                    formatter,
                    "result for {} answers no open tool use",
                    id.as_str()
                )
            }
            Self::UnansweredToolUse(id) => {
                write!(
                    formatter,
                    "tool use {} has no result in the next message",
                    id.as_str()
                )
            }
        }
    }
}

impl core::error::Error for MalformedRequest {}

/// The assistant's reply to a model call.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelResponse {
    pub content: Vec<ContentBlock>,
}

impl ModelResponse {
    /// Checks the response is something the session can act on.
    ///
    /// # Errors
    ///
    /// Returns why the response must be treated as a failed call instead.
    pub fn validate(&self) -> Result<(), InvalidResponse> {
        let mut seen = BTreeSet::new();
        for block in &self.content {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    if !seen.insert(id) {
                        return Err(InvalidResponse::DuplicateToolUse(id.clone()));
                    }
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    return Err(InvalidResponse::ContainsToolResult(tool_use_id.clone()));
                }
                ContentBlock::Text { .. } => {}
            }
        }
        Ok(())
    }

    /// The tool uses in the response, in order.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => Some(ToolCall {
                    tool_use_id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }),
                ContentBlock::Text { .. } | ContentBlock::ToolResult { .. } => None,
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidResponse {
    DuplicateToolUse(ToolUseId),
    ContainsToolResult(ToolUseId),
}

impl fmt::Display for InvalidResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateToolUse(id) => {
                write!(formatter, "response reuses tool use id {}", id.as_str())
            }
            Self::ContainsToolResult(id) => {
                write!(
                    formatter,
                    "response contains a tool result for {}",
                    id.as_str()
                )
            }
        }
    }
}

impl core::error::Error for InvalidResponse {}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelOutcome {
    Responded {
        response: ModelResponse,
    },
    /// The call failed or timed out. A retryable failure is tried again,
    /// within the session's retry policy, after at least `retry_after`.
    Failed {
        reason: String,
        retryable: bool,
        retry_after: Option<Millis>,
    },
    Cancelled,
}

/// How a session calls the model. Fixed when the session is created, so a
/// log replays the same way whatever the current configuration says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelPolicy {
    /// How long a call may take, from its intent, before it counts as failed.
    pub timeout: Millis,
    pub retry: RetryPolicy,
}

/// Exponential backoff between attempts at one model call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Attempts per call, counting the first. One means never retry.
    pub max_attempts: u32,
    pub initial_backoff: Millis,
    pub max_backoff: Millis,
}

impl RetryPolicy {
    /// The wait before attempt `failures + 1`, after `failures` consecutive
    /// failures, before any jitter.
    #[must_use]
    pub fn backoff(&self, failures: u32) -> Millis {
        let doublings = failures.saturating_sub(1);
        let factor = 2u64.checked_pow(doublings).unwrap_or(u64::MAX);
        Millis::new(
            self.initial_backoff
                .get()
                .saturating_mul(factor)
                .min(self.max_backoff.get()),
        )
    }
}
