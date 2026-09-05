//! # ah-plugins-rails
//!
//! 真实 rails:注册为 tools/pre-execute waterfall 监听器。
//! 当前实现:
//! - ShellGuardRail:拒绝 run_shell 工具执行危险命令模式(rm -rf / mkfs / dd if= / fork bomb);
//! - PathGuardRail:拒绝 fs 工具(path 参数)使用绝对路径或 .. 逃逸;
//! - ToolBudgetRail:限制工具调用总次数,超限拒绝;
//! - ApprovalRail(渐进披露):未批准工具被拒,批准集持久化(tool-approval seam)。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ah_contracts::goal::{
    GoalAssessment, GoalAssessmentStatus, GoalError, GoalRecord, GoalRuntime, GoalStatus,
};
use ah_contracts::harness_schema::CompletionPromiseEvaluator;
use ah_contracts::keys::{GOAL_MANAGER, RAILS, TOOL_APPROVAL};
use ah_contracts::prelude::Effect;
use ah_contracts::rails::{RailConfig, RailDecision, RailInput, RailPhase, RailRuntime};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tool_approval::{ToolApproval, ToolApprovalError};
use ah_contracts::tools::{ToolDecision, ToolInvocation};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 检测与 Python `LLMRetryRail` 相同阈值的重复流后缀。
pub fn detect_repeated_suffix(text: &str) -> Option<(String, usize)> {
    const MIN_PATTERN_CHARS: usize = 2;
    const MAX_PATTERN_CHARS: usize = 64;
    const MIN_COUNT: usize = 6;
    const MIN_TOTAL_CHARS: usize = 160;
    const WINDOW_CHARS: usize = 1024;
    const SINGLE_CHAR_COUNT: usize = 100;

    let chars: Vec<char> = text.chars().collect();
    let start = chars.len().saturating_sub(WINDOW_CHARS);
    let tail = &chars[start..];
    if let Some(&last) = tail.last()
        && !last.is_whitespace()
    {
        let mut count = 1;
        for &current in tail[..tail.len() - 1].iter().rev() {
            if current != last {
                break;
            }
            count += 1;
            if count >= SINGLE_CHAR_COUNT {
                return Some((last.to_string(), count));
            }
        }
    }

    let max_len = MAX_PATTERN_CHARS.min(tail.len() / MIN_COUNT);
    for unit_len in MIN_PATTERN_CHARS..=max_len {
        let unit = &tail[tail.len() - unit_len..];
        if unit.iter().all(|character| character.is_whitespace())
            || unit.windows(2).all(|pair| pair[0] == pair[1])
        {
            continue;
        }
        let required_count = MIN_COUNT.max(MIN_TOTAL_CHARS.div_ceil(unit_len));
        let mut count = 1;
        let mut end = tail.len() - unit_len;
        while end >= unit_len && tail[end - unit_len..end] == *unit {
            count += 1;
            end -= unit_len;
        }
        if count >= required_count {
            return Some((unit.iter().collect(), count));
        }
    }
    None
}

/// 判断 Python `LLMRetryRail` 是否将错误识别为重复流输出。
pub fn is_llm_repeat_error(message: &str) -> bool {
    message.contains("LLM repeated stream output detected")
}

/// 判断 Python `LLMRetryRail` 是否将错误识别为流超时。
pub fn is_llm_stream_timeout_error(message: &str) -> bool {
    ["LLM stream timeout", "stream frame timeout"]
        .iter()
        .any(|marker| message.contains(marker))
}

/// 返回 Python `LLMRetryRail.backoff_delay` 的毫秒表示。
pub fn llm_retry_backoff(backoff_ms: &[u64], retry_index: usize) -> u64 {
    backoff_ms
        .get(retry_index)
        .copied()
        .or_else(|| backoff_ms.last().copied())
        .unwrap_or(0)
}

/// 判断 Python `ToolCallResilienceRail` 是否允许重试该异常。
pub fn is_tool_retryable_error(error_type: &str, message: &str) -> bool {
    let type_name = error_type.to_ascii_lowercase();
    let text = message.to_ascii_lowercase();
    let non_retryable = [
        "valueerror",
        "typeerror",
        "keyerror",
        "attributeerror",
        "permissionerror",
        "filenotfounderror",
        "isadirectoryerror",
    ];
    if non_retryable.iter().any(|name| type_name == *name) {
        return false;
    }
    let retryable_types = [
        "timeouterror",
        "connectionreseterror",
        "brokenpipeerror",
        "connectionabortederror",
    ];
    if retryable_types.iter().any(|name| type_name == *name) {
        return true;
    }
    [
        "timed out",
        "timeout",
        "servertimeouterror",
        "session terminated",
        "closedresourceerror",
        "brokenresourceerror",
        "endofstream",
        "stream closed",
        "connection closed",
        "remoteprotocolerror",
        "readerror",
        "writeerror",
        "not connected",
        "connection reset",
        "connectionreseterror",
        "connectionaborted",
        "connectionabortederror",
        "broken pipe",
        "brokenpipeerror",
    ]
    .iter()
    .any(|marker| type_name.contains(marker) || text.contains(marker))
}

/// 生成与 Python `build_completion_signal_section` 相同的 prompt 文本。
pub fn completion_signal_prompt(language: &str, promise: &str) -> String {
    if language == "en" {
        format!(
            "\n\n## Completion Signal\nWhen the task is fully completed, output <promise>{promise}</promise> as the final line of your response. Do not output this tag until you are confident the task is complete."
        )
    } else {
        format!(
            "\n\n## 完成信号\n任务完全完成后，在回复的最后一行输出 <promise>{promise}</promise>。\n在确认任务完成前，不要输出此标签。"
        )
    }
}
/// 危险命令模式(真实、保守的阻止清单)。
pub const DANGEROUS_PATTERNS: &[&str] = &["rm -rf", "mkfs", "dd if=", ":(){"];

/// Shell 守卫 rail:拦截 run_shell 的危险命令。
pub struct ShellGuardRailPlugin;

/// 判断 run_shell 请求是否命中危险模式。
pub fn is_dangerous(command: &str) -> Option<&'static str> {
    DANGEROUS_PATTERNS
        .iter()
        .find(|pattern| command.contains(**pattern))
        .copied()
}

impl Plugin for ShellGuardRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let effect = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |event, decision, next| async move {
                if event.name == "run_shell"
                    && let Some(command) = decision.arguments.get("command").and_then(Value::as_str)
                    && let Some(pattern) = is_dangerous(command)
                {
                    return ToolDecision::deny(
                        decision.arguments,
                        format!("dangerous command pattern: {pattern}"),
                    );
                }
                next.next(decision).await
            },
        );
        Ok(vec![effect])
    }
}

/// 判断路径是否逃逸 workspace(绝对路径或含 .. 段)。
pub fn path_escapes_workspace(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    path.split('/').any(|segment| segment == "..")
}

/// 带 path 参数的 fs 工具(PathGuard 作用域)。
pub const PATH_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "list_dir",
    "remove_file",
    "edit",
    "glob",
    "grep",
];

/// 路径守卫 rail:拦截 fs 工具对 workspace 外路径的访问。
pub struct PathGuardRailPlugin;

impl Plugin for PathGuardRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-path"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let effect = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |event, decision, next| async move {
                let path = decision
                    .arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if PATH_TOOLS.contains(&event.name.as_str())
                    && let Some(path) = path
                    && path_escapes_workspace(&path)
                {
                    return ToolDecision::deny(
                        decision.arguments,
                        format!("path escapes workspace: {path}"),
                    );
                }
                next.next(decision).await
            },
        );
        Ok(vec![effect])
    }
}

/// 工具预算 rail:限制工具调用总次数(真实计数,跨会话累计)。
pub struct ToolBudgetRailPlugin {
    max_calls: usize,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ToolBudgetRailPlugin {
    /// 以调用上限创建(真实原子计数)。
    pub fn new(max_calls: usize) -> Self {
        Self {
            max_calls,
            calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl Plugin for ToolBudgetRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-budget"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let max_calls = self.max_calls;
        let calls = self.calls.clone();
        let effect =
            ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(move |event, decision, next| {
                let calls = calls.clone();
                async move {
                    let used = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if used > max_calls {
                        return ToolDecision::deny(
                            decision.arguments,
                            format!("tool budget exceeded ({max_calls})"),
                        );
                    }
                    let _ = event;
                    next.next(decision).await
                }
            });
        Ok(vec![effect])
    }
}

/// 真实文件后端工具批准集(dir/approved.json)。
pub struct FileToolApproval {
    dir: PathBuf,
    approved: Mutex<std::collections::HashSet<String>>,
}

impl FileToolApproval {
    /// 打开(或创建)批准集;可预置基线工具。
    pub fn open(dir: impl Into<PathBuf>, baseline: &[&str]) -> Result<Self, ToolApprovalError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| ToolApprovalError(format!("create approval dir: {e}")))?;
        let path = dir.join("approved.json");
        let approved = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ToolApprovalError(format!("read approvals: {e}")))?;
            serde_json::from_str(&text).unwrap_or_else(|_| std::collections::HashSet::new())
        } else {
            let set: std::collections::HashSet<String> =
                baseline.iter().map(|s| s.to_string()).collect();
            let text = serde_json::to_string(&set).expect("serialize");
            std::fs::write(&path, text)
                .map_err(|e| ToolApprovalError(format!("write approvals: {e}")))?;
            set
        };
        Ok(Self {
            dir,
            approved: Mutex::new(approved),
        })
    }

    fn persist(&self, set: &std::collections::HashSet<String>) -> Result<(), ToolApprovalError> {
        let text = serde_json::to_string(set)
            .map_err(|e| ToolApprovalError(format!("serialize approvals: {e}")))?;
        std::fs::write(self.dir.join("approved.json"), text)
            .map_err(|e| ToolApprovalError(format!("write approvals: {e}")))
    }
}

impl Seam for FileToolApproval {}

impl ToolApproval for FileToolApproval {
    fn approve(&self, name: &str) -> Result<(), ToolApprovalError> {
        let mut set = self.approved.lock().unwrap();
        set.insert(name.to_string());
        self.persist(&set)
    }

    fn revoke(&self, name: &str) -> Result<(), ToolApprovalError> {
        let mut set = self.approved.lock().unwrap();
        set.remove(name);
        self.persist(&set)
    }

    fn is_approved(&self, name: &str) -> bool {
        self.approved.lock().unwrap().contains(name)
    }

    fn approved(&self) -> Vec<String> {
        let mut names: Vec<String> = self.approved.lock().unwrap().iter().cloned().collect();
        names.sort();
        names
    }
}

/// 渐进披露 rail:未批准工具在 pre-execute 被拒;消费 tool-approval seam。
pub struct ApprovalRailPlugin {
    dir: PathBuf,
    baseline: Vec<&'static str>,
}

impl ApprovalRailPlugin {
    /// 以批准集目录与基线工具创建。
    pub fn new(dir: impl Into<PathBuf>, baseline: &[&'static str]) -> Self {
        Self {
            dir: dir.into(),
            baseline: baseline.to_vec(),
        }
    }
}

impl Plugin for ApprovalRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-approval"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TOOL_APPROVAL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let approval =
            FileToolApproval::open(&self.dir, &self.baseline).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let approval: Arc<dyn ToolApproval> = Arc::new(approval);
        let approval_clone = approval.clone();
        let effect =
            ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(move |event, decision, next| {
                let approval = approval_clone.clone();
                async move {
                    if !approval.is_approved(&event.name) {
                        return ToolDecision::deny(
                            decision.arguments,
                            format!("tool not approved: {}", event.name),
                        );
                    }
                    next.next(decision).await
                }
            });
        Ok(vec![ctx.register(TOOL_APPROVAL, approval), effect])
    }
}

struct RailSessionState {
    completion: Option<CompletionPromiseEvaluator>,
    content_stream: String,
    reasoning_stream: String,
}

pub struct LocalRailRuntime {
    config: RailConfig,
    sessions: Mutex<HashMap<String, RailSessionState>>,
}

impl LocalRailRuntime {
    pub fn new(config: RailConfig) -> Result<Self, ah_contracts::rails::RailError> {
        if config.required_confirmations == 0 {
            return Err(ah_contracts::rails::RailError(
                "required_confirmations must be >= 1".to_string(),
            ));
        }
        if config.retry_backoff_ms.is_empty() {
            return Err(ah_contracts::rails::RailError(
                "retry_backoff_ms must not be empty".to_string(),
            ));
        }
        Ok(Self {
            config,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    fn state_for<'a>(
        &'a self,
        sessions: &'a mut HashMap<String, RailSessionState>,
        session_id: &str,
    ) -> &'a mut RailSessionState {
        let promise = self.config.completion_promise.as_ref().map(|value| {
            CompletionPromiseEvaluator::new(value, self.config.required_confirmations)
        });
        sessions
            .entry(session_id.to_string())
            .or_insert(RailSessionState {
                completion: promise,
                content_stream: String::new(),
                reasoning_stream: String::new(),
            })
    }

    fn retry_delay(&self, attempt: u32) -> u64 {
        llm_retry_backoff(&self.config.retry_backoff_ms, attempt as usize)
    }

    fn model_error_retryable(error: &str) -> bool {
        let lower = error.to_ascii_lowercase();
        [
            "timeout",
            "timed out",
            "connection reset",
            "connection refused",
            "connection closed",
            "broken pipe",
            "stream closed",
            "transport",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn completion_body(output: &str) -> Option<String> {
        let lower = output.to_ascii_lowercase();
        let start = lower.find("<promise>")? + "<promise>".len();
        let end = lower[start..].find("</promise>")? + start;
        Some(output[start..end].trim().to_string())
    }

    fn normalize(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn promise_matches(&self, body: &str) -> bool {
        let Some(expected) = self.config.completion_promise.as_deref() else {
            return false;
        };
        let expected = Self::normalize(expected);
        let lines: Vec<String> = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(Self::normalize)
            .collect();
        let Some(first) = lines.first() else {
            return false;
        };
        first == &expected
            || (self.config.allow_promise_details && first.starts_with(&(expected + " ")))
    }
}

impl Seam for LocalRailRuntime {}

impl RailRuntime for LocalRailRuntime {
    fn config(&self) -> RailConfig {
        self.config.clone()
    }

    fn evaluate(&self, input: RailInput) -> RailDecision {
        let mut sessions = self.sessions.lock().unwrap();
        let state = self.state_for(&mut sessions, &input.session_id);
        match input.phase {
            RailPhase::BeforeIteration => {
                if self
                    .config
                    .max_rounds
                    .is_some_and(|max| input.iteration >= max)
                {
                    RailDecision::stop("maximum task-loop rounds reached")
                } else {
                    RailDecision::continue_()
                }
            }
            RailPhase::BeforeModelCall => {
                state.content_stream.clear();
                state.reasoning_stream.clear();
                RailDecision::continue_()
            }
            RailPhase::ModelChunk => {
                let delta = input.stream_delta.as_deref().unwrap_or_default();
                let stream = match input.stream_field.as_deref() {
                    Some("reasoning_content") | Some("reasoning") => &mut state.reasoning_stream,
                    Some("content") => &mut state.content_stream,
                    _ => return RailDecision::continue_(),
                };
                stream.push_str(delta);
                if let Some((unit, count)) = detect_repeated_suffix(stream) {
                    if input.attempt < self.config.max_model_retries {
                        return RailDecision::retry(
                            self.retry_delay(input.attempt),
                            format!(
                                "LLM repeated stream output detected: unit={unit:?}, repeat_count={count}"
                            ),
                        );
                    }
                    return RailDecision::deny(format!(
                        "LLM repeated stream output detected after retries: unit={unit:?}, repeat_count={count}"
                    ));
                }
                RailDecision::continue_()
            }
            RailPhase::ModelOutput => {
                let Some(body) = input.output.as_deref().and_then(Self::completion_body) else {
                    if let Some(completion) = &mut state.completion {
                        completion.notify_absent();
                    }
                    return RailDecision::continue_();
                };
                if !self.promise_matches(&body) {
                    if let Some(completion) = &mut state.completion {
                        completion.notify_absent();
                    }
                    return RailDecision::continue_();
                }
                if let Some(completion) = &mut state.completion {
                    completion.notify_fulfilled(&body);
                    if completion.should_stop(&Default::default()) {
                        return RailDecision::stop("completion promise fulfilled");
                    }
                }
                RailDecision::continue_()
            }
            RailPhase::ModelError => {
                let error = input.error.as_deref().unwrap_or("model call failed");
                if (Self::model_error_retryable(error)
                    || is_llm_repeat_error(error)
                    || is_llm_stream_timeout_error(error))
                    && input.attempt < self.config.max_model_retries
                {
                    RailDecision::retry(
                        self.retry_delay(input.attempt),
                        format!("retryable model error: {error}"),
                    )
                } else {
                    RailDecision::deny(format!("model retry policy denied: {error}"))
                }
            }
            RailPhase::ToolError => {
                let error = input.error.as_deref().unwrap_or("tool call failed");
                if !input.tool_idempotent {
                    return RailDecision::deny(format!(
                        "non-idempotent tool {} is not retryable",
                        input.tool_name.as_deref().unwrap_or("unknown")
                    ));
                }
                if is_tool_retryable_error("RuntimeError", error)
                    && input.attempt < self.config.max_tool_retries
                {
                    RailDecision::retry(
                        self.retry_delay(input.attempt),
                        format!("retryable tool error: {error}"),
                    )
                } else {
                    RailDecision::deny(format!("tool retry policy denied: {error}"))
                }
            }
        }
    }

    fn reset(&self, session_id: &str) {
        self.sessions.lock().unwrap().remove(session_id);
    }
}

/// In-memory session-scoped GoalManager.
///
/// The runtime owns every write and uses goal/revision matching to reject stale
/// attempts. Persistence remains the host's responsibility, like other local
/// rail state; callers can serialize the returned `GoalRecord`.
pub struct LocalGoalManager {
    goals: Mutex<HashMap<String, GoalRecord>>,
}

impl LocalGoalManager {
    pub fn new() -> Self {
        Self {
            goals: Mutex::new(HashMap::new()),
        }
    }

    fn matches(record: &GoalRecord, goal_id: &str, revision: u64) -> bool {
        record.goal_id == goal_id
            && record.revision == revision
            && matches!(record.status, GoalStatus::Active | GoalStatus::Paused)
    }
}

impl Default for LocalGoalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for LocalGoalManager {}

impl GoalRuntime for LocalGoalManager {
    fn get(&self, session_id: &str) -> Option<GoalRecord> {
        self.goals.lock().unwrap().get(session_id).cloned()
    }

    fn set(
        &self,
        session_id: &str,
        objective: &str,
        overwrite_confirmed: bool,
        max_attempts: Option<u64>,
        token_budget: Option<u64>,
    ) -> Result<GoalRecord, GoalError> {
        let objective = objective.trim();
        if objective.is_empty() {
            return Err(GoalError::new(
                "invalid_objective",
                "goal objective must not be empty",
            ));
        }
        if max_attempts.is_some_and(|value| value == 0)
            || token_budget.is_some_and(|value| value == 0)
        {
            return Err(GoalError::new(
                "invalid_objective",
                "goal limits must be positive",
            ));
        }
        let mut goals = self.goals.lock().unwrap();
        if goals.contains_key(session_id) && !overwrite_confirmed {
            return Err(GoalError::new(
                "already_exists",
                "a goal already exists for this session",
            ));
        }
        let record = GoalRecord::create(session_id, objective, max_attempts, token_budget);
        goals.insert(session_id.to_string(), record.clone());
        Ok(record)
    }

    fn pause(&self, session_id: &str) -> Option<GoalRecord> {
        let mut goals = self.goals.lock().unwrap();
        let record = goals.get_mut(session_id)?;
        if record.status == GoalStatus::Active {
            record.settle_time(false);
            record.status = GoalStatus::Paused;
            record.touch(false);
        }
        Some(record.clone())
    }

    fn resume(&self, session_id: &str) -> Option<GoalRecord> {
        let mut goals = self.goals.lock().unwrap();
        let record = goals.get_mut(session_id)?;
        if matches!(record.status, GoalStatus::Paused | GoalStatus::Blocked) {
            record.status = GoalStatus::Active;
            record.revision = record.revision.saturating_add(1);
            record.settle_time(true);
        }
        Some(record.clone())
    }

    fn clear(&self, session_id: &str) -> Option<GoalRecord> {
        self.goals.lock().unwrap().remove(session_id)
    }

    fn begin_attempt(&self, session_id: &str, goal_id: &str, revision: u64) -> Option<GoalRecord> {
        let mut goals = self.goals.lock().unwrap();
        let record = goals.get_mut(session_id)?;
        if !Self::matches(record, goal_id, revision) || record.status != GoalStatus::Active {
            return None;
        }
        record.attempt_count = record.attempt_count.saturating_add(1);
        record.settle_time(true);
        Some(record.clone())
    }

    fn accumulate_usage(
        &self,
        session_id: &str,
        goal_id: &str,
        revision: u64,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
    ) {
        let mut goals = self.goals.lock().unwrap();
        let Some(record) = goals.get_mut(session_id) else {
            return;
        };
        if !Self::matches(record, goal_id, revision) {
            return;
        }
        record.settle_time(true);
        record
            .token_usage
            .accumulate(input_tokens, output_tokens, cached_input_tokens);
        record.touch(false);
    }

    fn apply_assessment(
        &self,
        session_id: &str,
        goal_id: &str,
        revision: u64,
        assessment: GoalAssessment,
    ) -> Option<GoalRecord> {
        let mut goals = self.goals.lock().unwrap();
        let record = goals.get_mut(session_id)?;
        if !Self::matches(record, goal_id, revision) {
            return None;
        }
        record.last_assessment = Some(assessment.clone());
        match assessment.status {
            GoalAssessmentStatus::Complete => {
                record.settle_time(false);
                record.status = GoalStatus::Completed;
                record.last_stop_reason = Some("completed".into());
            }
            GoalAssessmentStatus::Blocked => {
                record.settle_time(false);
                record.status = GoalStatus::Blocked;
                record.last_stop_reason = Some("blocked".into());
            }
            GoalAssessmentStatus::Continue => {
                let keep_active = record.status == GoalStatus::Active;
                record.settle_time(keep_active);
            }
        }
        record.touch(false);
        Some(record.clone())
    }
}

/// Plugin exposing the session-scoped GoalManager seam.
pub struct GoalPlugin;

impl Plugin for GoalPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-goal"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![GOAL_MANAGER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        Ok(vec![ctx.register(
            GOAL_MANAGER,
            Arc::new(LocalGoalManager::new()) as Arc<dyn GoalRuntime>,
        )])
    }
}

pub struct TaskPolicyRailPlugin {
    config: RailConfig,
}

impl TaskPolicyRailPlugin {
    pub fn new(config: RailConfig) -> Self {
        Self { config }
    }
}

impl Default for TaskPolicyRailPlugin {
    fn default() -> Self {
        Self::new(RailConfig::default())
    }
}

impl Plugin for TaskPolicyRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-policy"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RAILS]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let runtime: Arc<dyn RailRuntime> =
            Arc::new(LocalRailRuntime::new(self.config.clone()).map_err(|error| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: error.0,
                }
            })?);
        Ok(vec![ctx.register(RAILS, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TOOLS;
    use ah_contracts::rails::RailAction;
    use ah_contracts::tools::ToolRegistry;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc;
    #[test]
    fn goal_manager_enforces_session_generation_and_completion() {
        let manager = LocalGoalManager::new();
        let goal = manager
            .set("s1", "ship it", false, Some(2), Some(100))
            .expect("set goal");
        assert_eq!(manager.get("s1").unwrap().status, GoalStatus::Active);
        assert_eq!(
            manager
                .set("s1", "replace", false, None, None)
                .unwrap_err()
                .code,
            "already_exists"
        );
        let started = manager
            .begin_attempt("s1", &goal.goal_id, goal.revision)
            .expect("begin attempt");
        manager.accumulate_usage("s1", &goal.goal_id, started.revision, 3, 4, 1);
        let completed = manager
            .apply_assessment(
                "s1",
                &goal.goal_id,
                started.revision,
                GoalAssessment {
                    status: GoalAssessmentStatus::Complete,
                    evidence: "verified".into(),
                    remaining_work: None,
                    next_instruction: None,
                },
            )
            .expect("complete goal");
        assert_eq!(completed.status, GoalStatus::Completed);
        assert_eq!(completed.attempt_count, 1);
        assert_eq!(completed.token_usage.total_tokens, 7);
        assert!(
            manager
                .begin_attempt("s1", &goal.goal_id, goal.revision)
                .is_none()
        );
    }

    #[test]
    fn goal_manager_pause_resume_bumps_idle_generation() {
        let manager = LocalGoalManager::new();
        let goal = manager
            .set("s1", "write report", false, None, None)
            .unwrap();
        let paused = manager.pause("s1").unwrap();
        assert_eq!(paused.status, GoalStatus::Paused);
        let resumed = manager.resume("s1").unwrap();
        assert_eq!(resumed.status, GoalStatus::Active);
        assert_eq!(resumed.revision, goal.revision + 1);
        assert!(
            manager
                .begin_attempt("s1", &goal.goal_id, goal.revision)
                .is_none()
        );
        assert!(manager.clear("s1").is_some());
        assert!(manager.get("s1").is_none());
    }

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            Arc::new(ShellGuardRailPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn shell_guard_allows_safe_command() {
        let root = std::env::temp_dir().join(format!("ah-rails-safe-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let output = registry
            .invoke("run_shell", json!({ "command": "echo", "args": ["safe"] }))
            .await
            .expect("safe command should run");
        assert_eq!(output["exit_code"], 0);
        assert!(
            output["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("safe")
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn shell_guard_blocks_dangerous_command() {
        let root = std::env::temp_dir().join(format!("ah-rails-danger-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let error = registry
            .invoke("run_shell", json!({ "command": "rm -rf /" }))
            .await
            .expect_err("dangerous command must be blocked");
        assert!(error.0.contains("dangerous command pattern"));
        assert!(error.0.contains("rm -rf"));

        // 拒绝后不产生副作用:workspace 里什么都没变(工具未执行)。
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn shell_guard_does_not_affect_other_tools() {
        let root = std::env::temp_dir().join(format!("ah-rails-fs-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let written = registry
            .invoke("write_file", json!({ "path": "ok.txt", "content": "x" }))
            .await
            .expect("write_file should pass");
        assert_eq!(written["written"], 1);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dangerous_pattern_detection() {
        assert_eq!(is_dangerous("rm -rf /"), Some("rm -rf"));
        assert_eq!(is_dangerous("echo hi"), None);
        assert_eq!(is_dangerous("mkfs.ext4 /dev/sda"), Some("mkfs"));
    }

    #[test]
    fn path_escape_detection() {
        assert!(
            path_escapes_workspace("/etc/passwd"),
            "absolute path escapes"
        );
        assert!(
            path_escapes_workspace("../secret.txt"),
            "parent traversal escapes"
        );
        assert!(
            path_escapes_workspace("a/../../b"),
            "nested traversal escapes"
        );
        assert!(!path_escapes_workspace("ok.txt"));
        assert!(!path_escapes_workspace("a/b/c.txt"));
    }

    #[tokio::test]
    async fn path_guard_blocks_escapes_but_allows_safe() {
        let root = std::env::temp_dir().join(format!("ah-rails-path-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(PathGuardRailPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        // 逃逸路径被 rails 拒绝(工具未执行)。
        let err = registry
            .invoke("read_file", json!({ "path": "/etc/passwd" }))
            .await
            .expect_err("absolute path must be blocked");
        assert!(err.0.contains("path escapes workspace"));
        let err = registry
            .invoke("read_file", json!({ "path": "../secret.txt" }))
            .await
            .expect_err("parent traversal must be blocked");
        assert!(err.0.contains("path escapes workspace"));

        for (name, arguments) in [
            (
                "edit",
                json!({ "path": "../secret.txt", "old_string": "x" }),
            ),
            ("glob", json!({ "path": "../", "pattern": "*" })),
            ("grep", json!({ "path": "/etc", "pattern": "root" })),
        ] {
            let err = registry
                .invoke(name, arguments)
                .await
                .expect_err("path escape must be blocked before tool execution");
            assert!(
                err.0.contains("path escapes workspace"),
                "{name}: {}",
                err.0
            );
        }

        // 安全路径正常执行。
        registry
            .invoke("write_file", json!({ "path": "ok.txt", "content": "x" }))
            .await
            .expect("safe write passes");
        let output = registry
            .invoke("read_file", json!({ "path": "ok.txt" }))
            .await
            .expect("safe read passes");
        assert_eq!(output["content"], "x");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn budget_rail_blocks_after_limit() {
        let root = std::env::temp_dir().join(format!("ah-rails-budget-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(ToolBudgetRailPlugin::new(2)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect("call 1 within budget");
        registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect("call 2 within budget");
        let err = registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect_err("call 3 over budget");
        assert!(err.0.contains("tool budget exceeded"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn approval_rail_denies_unapproved_and_allows_after_approve() {
        let root = std::env::temp_dir().join(format!("ah-rails-appr-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(ApprovalRailPlugin::new(root.join("approvals"), &[])),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        // 未批准:拒绝(真实 pre-execute)。
        let err = registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect_err("unapproved tool denied");
        assert!(err.0.contains("tool not approved"));

        // 批准后:放行;批准集真实落盘。
        let approval = ctx
            .service::<dyn ToolApproval>(&TOOL_APPROVAL)
            .expect("approval");
        assert!(!approval.is_approved("list_dir"));
        approval.approve("list_dir").expect("approve");
        assert!(approval.is_approved("list_dir"));
        assert!(
            root.join("approvals").join("approved.json").exists(),
            "persisted"
        );

        registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect("approved tool allowed");

        // 撤销后再次拒绝。
        approval.revoke("list_dir").expect("revoke");
        assert!(
            registry
                .invoke("list_dir", json!({ "path": "." }))
                .await
                .is_err(),
            "revoked tool denied again"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn approval_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-rails-appr2-{}", std::process::id()));
        let (provider, _) = {
            let p = FileToolApproval::open(root.join("approvals"), &["list_dir"]).expect("open");
            p.approve("run_shell").expect("approve");
            (p, ())
        };
        let _ = provider;
        let reopened = FileToolApproval::open(root.join("approvals"), &[]).expect("reopen");
        assert!(reopened.is_approved("list_dir"), "baseline persisted");
        assert!(reopened.is_approved("run_shell"), "approval persisted");
        let _ = std::fs::remove_dir_all(&root);
    }
    fn rail_input(session: &str, phase: RailPhase) -> RailInput {
        RailInput::new(session, phase)
    }

    #[test]
    fn completion_requires_consecutive_confirmations_and_resets_per_session() {
        let runtime = LocalRailRuntime::new(RailConfig {
            completion_promise: Some("done".to_string()),
            required_confirmations: 2,
            ..Default::default()
        })
        .expect("runtime");
        let mut first = rail_input("s1", RailPhase::ModelOutput);
        first.output = Some("<promise>done</promise>".to_string());
        assert_eq!(runtime.evaluate(first.clone()).action, RailAction::Continue);
        let mut absent = rail_input("s1", RailPhase::ModelOutput);
        absent.output = Some("still working".to_string());
        assert_eq!(runtime.evaluate(absent).action, RailAction::Continue);
        assert_eq!(runtime.evaluate(first.clone()).action, RailAction::Continue);
        assert_eq!(runtime.evaluate(first).action, RailAction::Stop);

        let mut other = rail_input("s2", RailPhase::ModelOutput);
        other.output = Some("<PROMISE>done</PROMISE>".to_string());
        assert_eq!(runtime.evaluate(other.clone()).action, RailAction::Continue);
        runtime.reset("s2");
        assert_eq!(runtime.evaluate(other).action, RailAction::Continue);
    }

    #[test]
    fn completion_details_and_round_limit_are_explicit() {
        let runtime = LocalRailRuntime::new(RailConfig {
            max_rounds: Some(3),
            completion_promise: Some("all checks pass".to_string()),
            allow_promise_details: true,
            ..Default::default()
        })
        .expect("runtime");

        let mut round = rail_input("s", RailPhase::BeforeIteration);
        round.iteration = 3;
        assert_eq!(runtime.evaluate(round).action, RailAction::Stop);

        let mut output = rail_input("s", RailPhase::ModelOutput);
        output.output = Some(
            "<promise>\nall   checks pass with coverage\nextra detail\n</promise>".to_string(),
        );
        assert_eq!(runtime.evaluate(output).action, RailAction::Stop);
    }
    #[test]
    fn llm_retry_classifies_error_markers_and_clamps_backoff() {
        assert!(is_llm_repeat_error(
            "LLM repeated stream output detected: field=content"
        ));
        assert!(is_llm_stream_timeout_error("stream frame timeout"));
        assert!(is_llm_stream_timeout_error("LLM stream timeout"));
        assert!(!is_llm_repeat_error("business failure"));
        assert!(!is_llm_stream_timeout_error("business failure"));
        assert_eq!(llm_retry_backoff(&[500, 1000, 2000], 0), 500);
        assert_eq!(llm_retry_backoff(&[500, 1000, 2000], 3), 2000);
        assert_eq!(llm_retry_backoff(&[], 0), 0);
    }

    #[test]
    fn retry_is_bounded_and_non_idempotent_tools_never_retry() {
        let runtime = LocalRailRuntime::new(RailConfig {
            max_model_retries: 2,
            max_tool_retries: 1,
            retry_backoff_ms: vec![10, 20],
            ..Default::default()
        })
        .expect("runtime");
        let mut model = rail_input("s", RailPhase::ModelError);
        model.error = Some("connection reset by peer".to_string());
        assert_eq!(
            runtime.evaluate(model.clone()),
            RailDecision::retry(10, "retryable model error: connection reset by peer")
        );
        model.attempt = 1;
        assert_eq!(runtime.evaluate(model.clone()).retry_after_ms, Some(20));
        model.attempt = 2;
        assert_eq!(runtime.evaluate(model).action, RailAction::Deny);

        let mut tool = rail_input("s", RailPhase::ToolError);
        tool.tool_name = Some("write_file".to_string());
        tool.error = Some("stream closed".to_string());
        assert_eq!(runtime.evaluate(tool.clone()).action, RailAction::Deny);
        tool.tool_idempotent = true;
        assert_eq!(runtime.evaluate(tool).action, RailAction::Retry);
    }

    #[test]
    fn llm_retry_detects_repeated_suffixes_with_python_thresholds() {
        let text = "xy".repeat(80);
        assert_eq!(detect_repeated_suffix(&text), Some(("xy".to_string(), 80)));

        assert_eq!(detect_repeated_suffix("normal output"), None);
    }
    #[test]
    fn tool_retry_matches_python_exception_classification() {
        assert!(is_tool_retryable_error("TimeoutError", ""));
        assert!(is_tool_retryable_error("RuntimeError", "stream closed"));
        assert!(!is_tool_retryable_error("ValueError", "bad argument"));
        assert!(!is_tool_retryable_error(
            "PermissionError",
            "permission denied"
        ));
    }

    #[test]
    fn stream_repeat_requests_bounded_retry_and_resets_per_model_call() {
        let runtime = LocalRailRuntime::new(RailConfig {
            max_model_retries: 1,
            retry_backoff_ms: vec![25, 50],
            ..Default::default()
        })
        .expect("runtime");
        assert_eq!(
            runtime
                .evaluate(RailInput::new("s", RailPhase::BeforeModelCall))
                .action,
            RailAction::Continue
        );
        let mut chunk = RailInput::new("s", RailPhase::ModelChunk);
        chunk.attempt = 0;
        chunk.stream_field = Some("content".into());
        chunk.stream_delta = Some("xy".repeat(80));
        assert_eq!(runtime.evaluate(chunk.clone()).action, RailAction::Retry);
        chunk.attempt = 1;
        assert_eq!(runtime.evaluate(chunk).action, RailAction::Deny);
        assert_eq!(
            runtime
                .evaluate(RailInput::new("s", RailPhase::BeforeModelCall))
                .action,
            RailAction::Continue
        );
    }
    #[test]
    fn invalid_policy_is_rejected_and_plugin_registers_runtime() {
        assert!(
            LocalRailRuntime::new(RailConfig {
                required_confirmations: 0,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            LocalRailRuntime::new(RailConfig {
                retry_backoff_ms: Vec::new(),
                ..Default::default()
            })
            .is_err()
        );

        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TaskPolicyRailPlugin::default());
        let effects = ctx.mount(&plugin).expect("mount policy");
        assert!(ctx.service::<dyn RailRuntime>(&RAILS).is_some());
        drop(effects);
        assert!(ctx.service::<dyn RailRuntime>(&RAILS).is_none());
    }
}
