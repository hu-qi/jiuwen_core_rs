//! Model backup/fallback seam.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::llm::{ModelChunk, ModelProvider, ModelRequest, ModelResponse};
use crate::seam::Seam;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelBackupError(pub String);

impl core::fmt::Display for ModelBackupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ModelBackupError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelBackupPolicy {
    pub attempt_timeout_ms: Option<u64>,
    pub retries_per_model: usize,
}

pub trait ModelBackupPolicyProvider: Seam {
    fn policy(&self) -> ModelBackupPolicy;
}

#[async_trait]
pub trait ModelBackup: Seam {
    async fn chat(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
    ) -> Result<ModelResponse, ModelBackupError>;

    async fn chat_with_policy(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
        _policy: ModelBackupPolicy,
    ) -> Result<ModelResponse, ModelBackupError> {
        self.chat(primary, request).await
    }
    async fn stream_chat_with_policy(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
        sink: mpsc::Sender<ModelChunk>,
        policy: ModelBackupPolicy,
    ) -> Result<(), ModelBackupError> {
        let response = self.chat_with_policy(primary, request, policy).await?;
        let chunk = ModelChunk {
            content_delta: response.content,
            reasoning_delta: response.reasoning_content.unwrap_or_default(),
            tool_call_deltas: response
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(index, call)| crate::llm::ToolCallDelta {
                    index,
                    id: Some(call.id),
                    name: Some(call.name),
                    arguments: call.arguments.to_string(),
                })
                .collect(),
            done: true,
        };
        sink.send(chunk)
            .await
            .map_err(|_| ModelBackupError("stream sink closed".into()))
    }

    fn models(&self) -> Vec<String>;
}
