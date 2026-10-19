use alloc::{collections::BTreeMap, string::String, vec::Vec};

use crate::{
    ContentBlock, EffectId, Message, ModelOutcome, ModelRequest, Record, RecordBody, RequestId,
    Role, ToolUseId,
};

/// The conversation as the model sees it: a pure fold over the log, so a
/// server and every attached client derive the same transcript.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Transcript {
    messages: Vec<Message>,
    /// Admitted inputs whose turn has not started yet.
    queued: BTreeMap<RequestId, String>,
    /// Tool uses still waiting for their result.
    awaiting: BTreeMap<EffectId, ToolUseId>,
}

impl Transcript {
    #[must_use]
    pub fn fold<'record, R>(records: R) -> Self
    where
        R: IntoIterator<Item = &'record Record>,
    {
        let mut transcript = Self::default();
        for record in records {
            transcript.apply(record);
        }
        transcript
    }

    pub fn apply(&mut self, record: &Record) {
        match &record.body {
            RecordBody::InputAdmitted { request_id, text } => {
                self.queued.insert(request_id.clone(), text.clone());
            }
            RecordBody::TurnStarted { request_id } => {
                if let Some(text) = self.queued.remove(request_id) {
                    self.push(Role::User, ContentBlock::Text { text });
                }
            }
            RecordBody::ModelResolved {
                outcome: ModelOutcome::Responded { response },
                ..
            } => {
                for block in &response.content {
                    self.push(Role::Assistant, block.clone());
                }
            }
            RecordBody::ToolIntent { call, .. } => {
                self.awaiting
                    .insert(EffectId::new(record.offset), call.tool_use_id.clone());
            }
            RecordBody::ToolResolved { effect, outcome } => {
                if let Some(tool_use_id) = self.awaiting.remove(effect) {
                    let (content, is_error) = outcome.as_tool_result();
                    self.push(
                        Role::User,
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        },
                    );
                }
            }
            RecordBody::SessionCreated { .. }
            | RecordBody::ModelIntent { .. }
            | RecordBody::ModelResolved { .. }
            | RecordBody::TurnCancelled { .. }
            | RecordBody::TurnEnded { .. } => {}
        }
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The request for the next model call.
    #[must_use]
    pub fn request(&self) -> ModelRequest {
        ModelRequest {
            messages: self.messages.clone(),
        }
    }

    /// Appends to the last message when the role matches, so roles alternate.
    fn push(&mut self, role: Role, block: ContentBlock) {
        match self.messages.last_mut() {
            Some(last) if last.role == role => last.content.push(block),
            Some(_) | None => self.messages.push(Message {
                role,
                content: Vec::from([block]),
            }),
        }
    }
}
