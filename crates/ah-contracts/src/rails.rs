//! Agent-loop rail policy contract.
//!
//! This module carries only typed policy inputs/decisions. Providers live in
//! plugin crates; consumers (agent-loop, workflow, subagents) depend on this
//! seam rather than on a concrete rail implementation.

use crate::seam::Seam;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailPhase {
    BeforeIteration,
    BeforeModelCall,
    ModelChunk,
    ModelOutput,
    ModelError,
    ToolError,
}

#[derive(Debug, Clone)]
pub struct RailInput {
    pub session_id: String,
    pub phase: RailPhase,
    pub iteration: usize,
    pub attempt: u32,
    pub output: Option<String>,
    pub error: Option<String>,
    pub tool_name: Option<String>,
    pub tool_idempotent: bool,
    pub stream_field: Option<String>,
    pub stream_delta: Option<String>,
}

impl RailInput {
    pub fn new(session_id: impl Into<String>, phase: RailPhase) -> Self {
        Self {
            session_id: session_id.into(),
            phase,
            iteration: 0,
            attempt: 0,
            output: None,
            error: None,
            tool_name: None,
            tool_idempotent: false,
            stream_field: None,
            stream_delta: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailAction {
    Continue,
    Stop,
    Retry,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailDecision {
    pub action: RailAction,
    pub reason: Option<String>,
    pub retry_after_ms: Option<u64>,
}

impl RailDecision {
    pub fn continue_() -> Self {
        Self {
            action: RailAction::Continue,
            reason: None,
            retry_after_ms: None,
        }
    }

    pub fn stop(reason: impl Into<String>) -> Self {
        Self {
            action: RailAction::Stop,
            reason: Some(reason.into()),
            retry_after_ms: None,
        }
    }

    pub fn retry(after_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            action: RailAction::Retry,
            reason: Some(reason.into()),
            retry_after_ms: Some(after_ms),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            action: RailAction::Deny,
            reason: Some(reason.into()),
            retry_after_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailConfig {
    pub planning_prompt: Option<String>,
    pub max_rounds: Option<usize>,
    pub completion_promise: Option<String>,
    pub required_confirmations: u64,
    pub allow_promise_details: bool,
    pub max_model_retries: u32,
    pub max_tool_retries: u32,
    pub retry_backoff_ms: Vec<u64>,
}

impl Default for RailConfig {
    fn default() -> Self {
        Self {
            planning_prompt: None,
            max_rounds: None,
            completion_promise: None,
            required_confirmations: 1,
            allow_promise_details: false,
            max_model_retries: 0,
            max_tool_retries: 0,
            retry_backoff_ms: vec![100, 250, 500],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailError(pub String);

impl core::fmt::Display for RailError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RailError {}

/// Stateful policy provider consumed by agent execution runtimes.
pub trait RailRuntime: Seam {
    fn config(&self) -> RailConfig;
    fn evaluate(&self, input: RailInput) -> RailDecision;
    fn reset(&self, session_id: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_carry_structured_action_and_retry_delay() {
        assert_eq!(RailDecision::continue_().action, RailAction::Continue);
        assert_eq!(RailDecision::stop("done").reason.as_deref(), Some("done"));
        assert_eq!(
            RailDecision::retry(25, "transport").retry_after_ms,
            Some(25)
        );
        assert_eq!(RailDecision::deny("policy").action, RailAction::Deny);
    }

    #[test]
    fn default_policy_is_safe_and_does_not_retry() {
        let config = RailConfig::default();
        assert_eq!(config.required_confirmations, 1);
        assert_eq!(config.max_model_retries, 0);
        assert_eq!(config.max_tool_retries, 0);
        assert!(config.planning_prompt.is_none());
    }

    #[test]
    fn stream_input_carries_field_and_delta() {
        let mut input = RailInput::new("s", RailPhase::ModelChunk);
        input.stream_field = Some("content".into());
        input.stream_delta = Some("chunk".into());
        assert_eq!(input.stream_field.as_deref(), Some("content"));
        assert_eq!(input.stream_delta.as_deref(), Some("chunk"));
    }
}
