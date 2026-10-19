//! The vocabulary of piton.
//!
//! Everything here either appears in a session's durable log or crosses a
//! process boundary, plus pure projections over those values. Behaviour that
//! carries state from one step to the next lives in `piton-core`.
//!
//! Records are forever: a log written by any released version must keep
//! deserializing. The fixtures under `tests/fixtures` pin the wire format.

#![no_std]

extern crate alloc;

mod content;
mod id;
mod json;
mod model;
mod record;
mod tool;
mod transcript;

pub use content::{ContentBlock, Message, Role};
pub use id::{EffectId, Offset, RequestId, Timestamp, ToolName, ToolUseId, TurnId};
pub use json::{InvalidJson, Json};
pub use model::{InvalidResponse, MalformedRequest, ModelOutcome, ModelRequest, ModelResponse};
pub use record::{Record, RecordBody, SCHEMA_VERSION, TurnEnd};
pub use tool::{ReplayPolicy, ToolCall, ToolOutcome, ToolSpec};
pub use transcript::Transcript;
