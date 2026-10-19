use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Serialize};

use crate::{Json, ToolName, ToolUseId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

/// Provider-neutral conversation content. Provisional: thinking, images and
/// opaque provider blocks arrive with the first real model codec.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: ToolUseId,
        name: ToolName,
        input: Json,
    },
    ToolResult {
        tool_use_id: ToolUseId,
        content: Json,
        is_error: bool,
    },
}
