//! Goal lifecycle contract shared by the goal manager and rails consumers.

use crate::seam::Seam;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static GOAL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalAssessmentStatus {
    Continue,
    Complete,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalAssessment {
    pub status: GoalAssessmentStatus,
    pub evidence: String,
    #[serde(default)]
    pub remaining_work: Option<String>,
    #[serde(default)]
    pub next_instruction: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub total_tokens: u64,
}

impl GoalTokenUsage {
    pub fn accumulate(&mut self, input: u64, output: u64, cached_input: u64) {
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.cached_input_tokens = self.cached_input_tokens.saturating_add(cached_input);
        self.total_tokens = self
            .total_tokens
            .saturating_add(input.saturating_add(output));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRecord {
    pub goal_id: String,
    pub session_id: String,
    pub objective: String,
    pub status: GoalStatus,
    pub revision: u64,
    pub attempt_count: u64,
    pub token_usage: GoalTokenUsage,
    #[serde(default)]
    pub max_attempts: Option<u64>,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub last_assessment: Option<GoalAssessment>,
    #[serde(default)]
    pub last_stop_reason: Option<String>,
    pub time_used_ms: u64,
    #[serde(default)]
    pub active_started_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl GoalRecord {
    pub fn create(
        session_id: impl Into<String>,
        objective: impl Into<String>,
        max_attempts: Option<u64>,
        token_budget: Option<u64>,
    ) -> Self {
        let now = now_ms();
        let serial = GOAL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            goal_id: format!("goal-{now}-{serial}"),
            session_id: session_id.into(),
            objective: objective.into(),
            status: GoalStatus::Active,
            revision: 0,
            attempt_count: 0,
            token_usage: GoalTokenUsage::default(),
            max_attempts,
            token_budget,
            last_assessment: None,
            last_stop_reason: None,
            time_used_ms: 0,
            active_started_ms: Some(now),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn settle_time(&mut self, keep_active: bool) {
        let now = now_ms();
        if let Some(started) = self.active_started_ms {
            self.time_used_ms = self
                .time_used_ms
                .saturating_add(now.saturating_sub(started));
            self.active_started_ms = if keep_active { Some(now) } else { None };
        } else if keep_active && self.status == GoalStatus::Active {
            self.active_started_ms = Some(now);
        }
        self.updated_at_ms = now;
    }

    pub fn touch(&mut self, bump_revision: bool) {
        self.updated_at_ms = now_ms();
        if bump_revision {
            self.revision = self.revision.saturating_add(1);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalError {
    pub code: String,
    pub message: String,
}

impl GoalError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl core::fmt::Display for GoalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GoalError {}

/// Session-scoped persistent-goal lifecycle.
pub trait GoalRuntime: Seam {
    fn get(&self, session_id: &str) -> Option<GoalRecord>;
    fn set(
        &self,
        session_id: &str,
        objective: &str,
        overwrite_confirmed: bool,
        max_attempts: Option<u64>,
        token_budget: Option<u64>,
    ) -> Result<GoalRecord, GoalError>;
    fn pause(&self, session_id: &str) -> Option<GoalRecord>;
    fn resume(&self, session_id: &str) -> Option<GoalRecord>;
    fn clear(&self, session_id: &str) -> Option<GoalRecord>;
    fn begin_attempt(&self, session_id: &str, goal_id: &str, revision: u64) -> Option<GoalRecord>;
    fn accumulate_usage(
        &self,
        session_id: &str,
        goal_id: &str,
        revision: u64,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
    );
    fn apply_assessment(
        &self,
        session_id: &str,
        goal_id: &str,
        revision: u64,
        assessment: GoalAssessment,
    ) -> Option<GoalRecord>;
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_record_starts_active_with_timing() {
        let record = GoalRecord::create("s1", "ship it", Some(2), Some(100));
        assert_eq!(record.status, GoalStatus::Active);
        assert_eq!(record.revision, 0);
        assert_eq!(record.attempt_count, 0);
        assert!(record.active_started_ms.is_some());
    }

    #[test]
    fn token_usage_accumulates_total_input_and_output_only() {
        let mut usage = GoalTokenUsage::default();
        usage.accumulate(3, 4, 2);
        usage.accumulate(5, 6, 1);
        assert_eq!(usage.input_tokens, 8);
        assert_eq!(usage.output_tokens, 10);
        assert_eq!(usage.cached_input_tokens, 3);
        assert_eq!(usage.total_tokens, 18);
    }
}
