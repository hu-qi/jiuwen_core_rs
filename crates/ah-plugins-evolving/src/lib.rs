//! # ah-plugins-evolving
//!
//! 真实 evolving 运行时(对应 openjiuwen/agent_evolving):
//! - 轨迹抽取:真实解析会话日志(session/event)——工具调用与结果配对、
//!   错误识别、迭代预算、完成标志;
//! - 评估:本地确定性判据必算(真实指标:完成度/错误/工具多样性/预算);
//!   LLM judge 可用时附加反馈(不可用原因显式记录,不静默);
//! - 优化:从评估问题按真实规则推导建议;LLM 建议可用时附加。

use std::collections::HashMap;
use std::sync::Arc;

use ah_contracts::evolving::{
    Evaluation, EvolvingError, EvolvingRuntime, Experience, Refinement, RefinementTarget,
    StepOutcome, Trajectory, TrajectoryStep, Verdict,
};
use ah_contracts::keys::{EVOLVING, LLM, SESSION_MANAGER};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEvent, SessionEventKind, SessionManager};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub mod archive;
pub mod checkpoint_types;
pub mod constant;
pub mod dataset;
pub mod draft_schema;
pub mod experience_manager;
pub mod experience_query;
pub mod experience_types;
pub mod from_conv;
pub mod lifecycle;
pub mod online_orchestrator;
pub mod prompts_sections;
pub mod protocols;
pub mod rebuild;
pub mod signal;
pub mod skill_creation;
pub mod skill_creation_sections;
pub mod store_projection;
pub mod submission;
pub mod team_signal;
pub mod tool_call_chain;
pub mod tool_metadata;
pub mod tracker;
pub mod trainer_progress;
pub mod trajectory_codec;
pub mod updates;
pub mod utils;

/// 真实 evolving 运行时:轨迹抽取 + 本地判据评估 + 优化建议。
pub struct EvolvingRuntimeImpl {
    llm: Arc<dyn ModelProvider>,
    manager: Arc<dyn SessionManager>,
    /// 经验持久化目录(experiences.jsonl)。
    experience_dir: std::path::PathBuf,
}

impl EvolvingRuntimeImpl {
    /// 构造运行时(需要 LLM 与 SessionManager seam;经验写入 dir)。
    pub fn new(
        llm: Arc<dyn ModelProvider>,
        manager: Arc<dyn SessionManager>,
        experience_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            llm,
            manager,
            experience_dir: experience_dir.into(),
        }
    }

    fn experience_path(&self) -> std::path::PathBuf {
        self.experience_dir.join("experiences.jsonl")
    }
}

impl Seam for EvolvingRuntimeImpl {}

/// 从 ToolResult 输出判断工具是否失败(真实规则:JSON 含 error 键或文本以 error 开头)。
fn output_signals_error(output: &str) -> bool {
    let trimmed = output.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && let Some(obj) = v.as_object()
    {
        return obj.contains_key("error");
    }
    trimmed.to_lowercase().starts_with("error") || trimmed.to_lowercase().starts_with("failed")
}

/// 抽取 LLM 输出中的严格 JSON(对象或数组):取首个起始括号到末个结束括号。
fn extract_json(text: &str, open: char, close: char) -> Option<String> {
    let start = text.find(open)?;
    let end = text.rfind(close)?;
    if end <= start {
        return None;
    }
    Some(text[start..=end].to_string())
}

#[async_trait]
impl EvolvingRuntime for EvolvingRuntimeImpl {
    fn extract_trajectory(
        &self,
        task: &str,
        events: &[SessionEvent],
    ) -> Result<Trajectory, EvolvingError> {
        let mut steps: Vec<TrajectoryStep> = Vec::new();
        // tool_call_id → 步骤索引(等待结果配对)。
        let mut pending: HashMap<String, usize> = HashMap::new();
        let mut finished = false;
        let mut last_iteration: u32 = 0;
        // 尚未归属迭代预算的步骤起始下标(AgentStep 描述刚完成的步骤)。
        let mut unassigned_from: usize = 0;

        for event in events {
            match event.kind {
                SessionEventKind::User | SessionEventKind::System => {}
                SessionEventKind::AgentStep => {
                    let iter = event
                        .payload
                        .get("iteration")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    last_iteration = iter;
                    for s in steps.iter_mut().skip(unassigned_from) {
                        s.budget_used = iter;
                    }
                    unassigned_from = steps.len();
                    finished = event
                        .payload
                        .get("done")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
                SessionEventKind::Assistant => {
                    if let Some(calls) = event.payload.get("tool_calls").and_then(Value::as_array) {
                        for call in calls {
                            let name = call
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("?")
                                .to_string();
                            let args = call.get("arguments").cloned().unwrap_or(Value::Null);
                            let id = call
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let idx = steps.len();
                            steps.push(TrajectoryStep {
                                seq: event.seq,
                                action: format!("call {name}({args})"),
                                tool: Some(name),
                                outcome: StepOutcome::Skipped,
                                error: None,
                                budget_used: last_iteration,
                            });
                            if !id.is_empty() {
                                pending.insert(id, idx);
                            }
                        }
                    } else if let Some(content) =
                        event.payload.get("content").and_then(Value::as_str)
                    {
                        steps.push(TrajectoryStep {
                            seq: event.seq,
                            action: content.to_string(),
                            tool: None,
                            outcome: StepOutcome::Success,
                            error: None,
                            budget_used: last_iteration,
                        });
                        unassigned_from = steps.len();
                    }
                }
                SessionEventKind::ToolResult => {
                    let id = event
                        .payload
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let output = event
                        .payload
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(&idx) = pending.get(id) {
                        if output_signals_error(output) {
                            steps[idx].outcome = StepOutcome::Error;
                            steps[idx].error = Some(output.to_string());
                        } else {
                            steps[idx].outcome = StepOutcome::Success;
                        }
                        pending.remove(id);
                    }
                }
            }
        }

        Ok(Trajectory {
            task: task.to_string(),
            steps,
            finished,
        })
    }

    fn extract_session(&self, task: &str, session_id: &str) -> Result<Trajectory, EvolvingError> {
        let log = self
            .manager
            .open(session_id)
            .map_err(|e| EvolvingError(format!("open session {session_id}: {e}")))?;
        let events = log.events();
        self.extract_trajectory(task, &events)
    }

    async fn evaluate(&self, trajectory: &Trajectory) -> Result<Evaluation, EvolvingError> {
        // ---- 本地确定性判据(真实指标) ----
        let mut score: f64 = 0.5;
        let mut strengths: Vec<String> = Vec::new();
        let mut issues: Vec<String> = Vec::new();

        if trajectory.steps.is_empty() {
            return Ok(Evaluation {
                verdict: Verdict::Fail,
                score: 0.1,
                strengths: vec![],
                issues: vec!["no trajectory steps".to_string()],
                feedback: "local criteria: empty trajectory".to_string(),
            });
        }

        if trajectory.finished {
            score += 0.2;
            strengths.push("task finished".to_string());
        } else {
            score -= 0.25;
            issues.push("task not finished".to_string());
        }

        let errors: Vec<&TrajectoryStep> = trajectory
            .steps
            .iter()
            .filter(|s| s.outcome == StepOutcome::Error)
            .collect();
        let skipped: Vec<&TrajectoryStep> = trajectory
            .steps
            .iter()
            .filter(|s| s.outcome == StepOutcome::Skipped)
            .collect();

        score -= (errors.len() as f64 * 0.1).min(0.5);
        if errors.is_empty() && skipped.is_empty() {
            score += 0.1;
            strengths.push("clean run".to_string());
        }
        for s in errors.iter().take(3) {
            let tool = s.tool.as_deref().unwrap_or("?");
            let err = s.error.as_deref().unwrap_or("unknown");
            issues.push(format!("tool {tool} errored: {err}"));
        }
        for s in skipped.iter().take(2) {
            let tool = s.tool.as_deref().unwrap_or("?");
            issues.push(format!("tool call without result: {tool}"));
        }

        let mut unique_tools: Vec<&str> = trajectory
            .steps
            .iter()
            .filter_map(|s| s.tool.as_deref())
            .collect();
        unique_tools.sort_unstable();
        unique_tools.dedup();
        if unique_tools.len() >= 2 {
            score += 0.1;
            strengths.push(format!("tool diversity ({} tools)", unique_tools.len()));
        } else if trajectory.steps.len() > 1 {
            issues.push("relies on a single tool".to_string());
        }

        let max_iter = trajectory
            .steps
            .iter()
            .map(|s| s.budget_used)
            .max()
            .unwrap_or(0);
        if max_iter <= 6 {
            strengths.push("budget efficient".to_string());
        } else if max_iter > 12 {
            score -= 0.1;
            issues.push("budget heavy".to_string());
        }

        let score = score.clamp(0.0, 1.0);
        let verdict = if score >= 0.75 {
            Verdict::Pass
        } else if score >= 0.4 {
            Verdict::NeedsWork
        } else {
            Verdict::Fail
        };

        // ---- LLM judge(可用时附加;不可用原因显式记录) ----
        let mut llm_feedback = String::new();
        let summary = json!({
            "task": trajectory.task,
            "steps": trajectory.steps.len(),
            "finished": trajectory.finished,
            "errors": errors.len(),
            "unique_tools": unique_tools.len(),
            "local_score": score,
        });
        let judge_request = ModelRequest {
            messages: vec![
                ChatMessage::new(
                    ChatRole::System,
                    "You are a strict evaluator. Reply with ONLY a JSON object: {\"verdict\":\"pass|needs_work|fail\",\"score\":0.0-1.0,\"feedback\":\"short\"}",
                ),
                ChatMessage::new(ChatRole::User, summary.to_string()),
            ],
            ..Default::default()
        };
        match self.llm.chat(judge_request).await {
            Ok(resp) => {
                if let Some(raw) = extract_json(&resp.content, '{', '}') {
                    match serde_json::from_str::<Value>(&raw) {
                        Ok(v) => {
                            let llm_score = v.get("score").and_then(Value::as_f64).unwrap_or(score);
                            let fb = v
                                .get("feedback")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            llm_feedback = fb;
                            if (llm_score - score).abs() > 0.3 {
                                issues.push("llm judge diverges from local metrics".to_string());
                            }
                        }
                        Err(e) => {
                            issues.push(format!("llm judge unparseable response: {e}"));
                        }
                    }
                } else {
                    issues.push("llm judge returned no JSON".to_string());
                }
            }
            Err(e) => {
                issues.push(format!("llm judge error: {e}"));
            }
        }

        let feedback = if llm_feedback.is_empty() {
            format!("local criteria score={score:.2}, verdict={verdict:?}")
        } else {
            format!("llm judge: {llm_feedback}; local score={score:.2}")
        };

        Ok(Evaluation {
            verdict,
            score,
            strengths,
            issues,
            feedback,
        })
    }

    async fn optimize(
        &self,
        trajectory: &Trajectory,
        evaluation: &Evaluation,
    ) -> Result<Vec<Refinement>, EvolvingError> {
        // 已通过且高分:无需优化。
        if evaluation.verdict == Verdict::Pass && evaluation.score >= 0.9 {
            return Ok(vec![]);
        }

        let mut refinements: Vec<Refinement> = Vec::new();
        let issues = &evaluation.issues;

        if issues.iter().any(|i| i.starts_with("task not finished")) {
            refinements.push(Refinement {
                target: RefinementTarget::Task,
                suggestion: "将任务拆分为可独立验证的子目标,逐个完成并给出证据".to_string(),
                rationale: "trajectory did not finish".to_string(),
                confidence: 0.8,
            });
        }
        if let Some(issue) = issues.iter().find(|i| i.starts_with("tool ")) {
            let tool = issue
                .trim_start_matches("tool ")
                .split_whitespace()
                .next()
                .unwrap_or("?");
            refinements.push(Refinement {
                target: RefinementTarget::Tools,
                suggestion: format!("为 {tool} 增加错误处理/输入校验/回退路径"),
                rationale: issue.clone(),
                confidence: 0.7,
            });
        }
        if issues.iter().any(|i| i.contains("single tool")) {
            refinements.push(Refinement {
                target: RefinementTarget::Workflow,
                suggestion: "引入检索/子代理等互补能力,避免单工具死循环".to_string(),
                rationale: "relies on a single tool".to_string(),
                confidence: 0.75,
            });
        }
        if issues.iter().any(|i| i.contains("budget heavy")) {
            refinements.push(Refinement {
                target: RefinementTarget::Prompt,
                suggestion: "在提示中给出明确终止判据,减少无效迭代".to_string(),
                rationale: "budget heavy".to_string(),
                confidence: 0.7,
            });
        }
        if evaluation.score < 0.5 {
            refinements.push(Refinement {
                target: RefinementTarget::Prompt,
                suggestion: "补充成功判据与边界条件,使模型少走弯路".to_string(),
                rationale: format!("score below 0.5 ({:.2})", evaluation.score),
                confidence: 0.65,
            });
        }
        if refinements.is_empty() {
            refinements.push(Refinement {
                target: RefinementTarget::Prompt,
                suggestion: "保留本轨迹为样本,轻微调整提示后重试".to_string(),
                rationale: "no specific defect detected".to_string(),
                confidence: 0.5,
            });
        }

        // ---- LLM 优化建议(可用时附加;不可用原因显式附加) ----
        let req = ModelRequest {
            messages: vec![
                ChatMessage::new(
                    ChatRole::System,
                    "You are an optimizer. Reply with ONLY a JSON array of objects: [{\"target\":\"task|prompt|workflow|tools\",\"suggestion\":\"...\",\"rationale\":\"...\",\"confidence\":0.0-1.0}]",
                ),
                ChatMessage::new(
                    ChatRole::User,
                    json!({
                        "task": trajectory.task,
                        "verdict": format!("{:?}", evaluation.verdict),
                        "score": evaluation.score,
                        "issues": issues,
                    })
                    .to_string(),
                ),
            ],
            ..Default::default()
        };
        match self.llm.chat(req).await {
            Ok(resp) => {
                if let Some(raw) = extract_json(&resp.content, '[', ']') {
                    match serde_json::from_str::<Value>(&raw) {
                        Ok(v) => {
                            if let Some(arr) = v.as_array() {
                                for item in arr {
                                    let target = match item
                                        .get("target")
                                        .and_then(Value::as_str)
                                        .unwrap_or("prompt")
                                    {
                                        "task" => RefinementTarget::Task,
                                        "workflow" => RefinementTarget::Workflow,
                                        "tools" => RefinementTarget::Tools,
                                        _ => RefinementTarget::Prompt,
                                    };
                                    refinements.push(Refinement {
                                        target,
                                        suggestion: item
                                            .get("suggestion")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_string(),
                                        rationale: item
                                            .get("rationale")
                                            .and_then(Value::as_str)
                                            .unwrap_or("llm suggested")
                                            .to_string(),
                                        confidence: item
                                            .get("confidence")
                                            .and_then(Value::as_f64)
                                            .unwrap_or(0.5),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            refinements.push(Refinement {
                                target: RefinementTarget::Prompt,
                                suggestion: "请人工复核:LLM 优化建议未能解析".to_string(),
                                rationale: format!("llm optimizer unparseable: {e}"),
                                confidence: 0.0,
                            });
                        }
                    }
                } else {
                    refinements.push(Refinement {
                        target: RefinementTarget::Prompt,
                        suggestion: "请人工复核:LLM 优化建议未返回 JSON".to_string(),
                        rationale: "llm optimizer returned no JSON".to_string(),
                        confidence: 0.0,
                    });
                }
            }
            Err(e) => {
                refinements.push(Refinement {
                    target: RefinementTarget::Prompt,
                    suggestion: "请人工复核:LLM 优化不可用".to_string(),
                    rationale: format!("llm optimizer error: {e}"),
                    confidence: 0.0,
                });
            }
        }

        Ok(refinements)
    }

    fn save_experience(&self, experience: &Experience) -> Result<(), EvolvingError> {
        std::fs::create_dir_all(&self.experience_dir)
            .map_err(|e| EvolvingError(format!("create experience dir: {e}")))?;
        let line = serde_json::to_string(experience)
            .map_err(|e| EvolvingError(format!("serialize experience: {e}")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.experience_path())
            .map_err(|e| EvolvingError(format!("open experience file: {e}")))?;
        use std::io::Write;
        writeln!(file, "{line}").map_err(|e| EvolvingError(format!("append experience: {e}")))?;
        Ok(())
    }

    fn load_experiences(&self) -> Result<Vec<Experience>, EvolvingError> {
        let path = self.experience_path();
        if !path.exists() {
            return Ok(vec![]);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| EvolvingError(format!("read experiences: {e}")))?;
        let mut experiences = Vec::new();
        for line in text.lines() {
            if let Ok(experience) = serde_json::from_str::<Experience>(line) {
                experiences.push(experience);
            }
        }
        Ok(experiences)
    }

    fn search_experiences(&self, query: &str) -> Result<Vec<Experience>, EvolvingError> {
        let query = query.to_lowercase();
        Ok(self
            .load_experiences()?
            .into_iter()
            .filter(|e| e.task.to_lowercase().contains(&query))
            .collect())
    }
}

/// evolving 插件:注入 LLM 与 SessionManager,提供 evolving seam。
pub struct EvolvingPlugin {
    experience_dir: std::path::PathBuf,
}

impl EvolvingPlugin {
    /// 以经验目录创建插件。
    pub fn new(experience_dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            experience_dir: experience_dir.into(),
        }
    }
}

impl Plugin for EvolvingPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-evolving"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![EVOLVING]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![LLM, SESSION_MANAGER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let llm = ctx
            .service::<dyn ModelProvider>(&LLM)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "llm seam not registered".to_string(),
            })?;
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "session-manager seam not registered".to_string(),
            })?;
        let runtime: Arc<dyn EvolvingRuntime> = Arc::new(EvolvingRuntimeImpl::new(
            llm,
            manager,
            self.experience_dir.clone(),
        ));
        Ok(vec![ctx.register(EVOLVING, runtime)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::EVOLVING;
    use ah_contracts::session::SessionLog;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(EvolvingPlugin::new(root.join("evolving"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    /// 写入一次完整的干净会话:工具调用成功 + 最终回答 + done。
    fn clean_session(log: &dyn SessionLog) {
        log.append(
            SessionEventKind::User,
            json!({"content": "list the workspace"}),
        )
        .expect("user");
        log.append(
            SessionEventKind::Assistant,
            json!({"tool_calls": [{"id": "c1", "name": "list_dir", "arguments": {"path": "."}}]}),
        )
        .expect("assistant call");
        log.append(
            SessionEventKind::ToolResult,
            json!({"tool_call_id": "c1", "output": "[\"a.txt\",\"b.txt\"]"}),
        )
        .expect("tool result");
        log.append(
            SessionEventKind::AgentStep,
            json!({"iteration": 1, "tool_calls": 1, "done": false}),
        )
        .expect("step1");
        log.append(
            SessionEventKind::Assistant,
            json!({"tool_calls": [{"id": "c2", "name": "read_file", "arguments": {"path": "a.txt"}}]}),
        )
        .expect("assistant call2");
        log.append(
            SessionEventKind::ToolResult,
            json!({"tool_call_id": "c2", "output": "file content"}),
        )
        .expect("tool result2");
        log.append(
            SessionEventKind::AgentStep,
            json!({"iteration": 2, "tool_calls": 2, "done": true}),
        )
        .expect("step2");
        log.append(
            SessionEventKind::Assistant,
            json!({"content": "mock final answer: found 2 files"}),
        )
        .expect("assistant final");
    }

    /// 失败会话:错误工具结果 + 未完成。
    fn failed_session(log: &dyn SessionLog) {
        log.append(
            SessionEventKind::User,
            json!({"content": "do the impossible"}),
        )
        .expect("user");
        log.append(
            SessionEventKind::Assistant,
            json!({"tool_calls": [{"id": "e1", "name": "run_shell", "arguments": {"cmd": "boom"}}]}),
        )
        .expect("assistant call");
        log.append(
            SessionEventKind::ToolResult,
            json!({"tool_call_id": "e1", "output": "error: command failed"}),
        )
        .expect("tool error");
        log.append(
            SessionEventKind::AgentStep,
            json!({"iteration": 3, "tool_calls": 1, "done": false}),
        )
        .expect("step");
    }

    #[tokio::test]
    async fn extract_trajectory_from_real_session_log() {
        let root = std::env::temp_dir().join(format!("ah-evolve-extract-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("s1").expect("create");
        clean_session(log.as_ref());

        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        let traj = runtime
            .extract_session("list workspace", "s1")
            .expect("extract");
        assert!(traj.finished);
        assert_eq!(traj.steps.len(), 3);
        // 工具步骤已配对为 Success。
        assert!(traj.steps.iter().all(|s| s.outcome == StepOutcome::Success));
        assert_eq!(traj.steps[0].tool.as_deref(), Some("list_dir"));
        assert_eq!(traj.steps[0].budget_used, 1);
        assert_eq!(traj.steps[1].tool.as_deref(), Some("read_file"));
        assert_eq!(traj.steps[1].budget_used, 2);
        assert!(traj.steps[2].tool.is_none());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn evaluate_failed_trajectory_is_fail() {
        let root = std::env::temp_dir().join(format!("ah-evolve-fail-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("s2").expect("create");
        failed_session(log.as_ref());

        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        let traj = runtime
            .extract_session("do the impossible", "s2")
            .expect("extract");
        let eval = runtime.evaluate(&traj).await.expect("evaluate");
        assert_eq!(eval.verdict, Verdict::Fail);
        assert!(eval.score < 0.4);
        assert!(eval.issues.iter().any(|i| i.contains("not finished")));
        assert!(eval.issues.iter().any(|i| i.contains("run_shell errored")));
        // LLM judge(dev 用 mock)显式记录了不可用原因,而非静默。
        assert!(eval.issues.iter().any(|i| i.contains("llm judge")));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn evaluate_clean_run_passes() {
        let root = std::env::temp_dir().join(format!("ah-evolve-pass-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("s3").expect("create");
        clean_session(log.as_ref());

        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        let traj = runtime
            .extract_session("list workspace", "s3")
            .expect("extract");
        let eval = runtime.evaluate(&traj).await.expect("evaluate");
        assert_eq!(eval.verdict, Verdict::Pass);
        assert!(eval.score >= 0.75);
        assert!(eval.strengths.iter().any(|s| s.contains("tool diversity")));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn optimize_derives_refinements_from_issues() {
        let root = std::env::temp_dir().join(format!("ah-evolve-opt-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("s4").expect("create");
        failed_session(log.as_ref());

        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        let traj = runtime
            .extract_session("do the impossible", "s4")
            .expect("extract");
        let eval = runtime.evaluate(&traj).await.expect("evaluate");
        let refs = runtime.optimize(&traj, &eval).await.expect("optimize");
        assert!(!refs.is_empty());
        // 失败轨迹:应含 Task 或 Tools 定向建议。
        assert!(
            refs.iter()
                .any(|r| r.target == RefinementTarget::Task || r.target == RefinementTarget::Tools)
        );
        // 高置信度的本地建议在前;LLM(不可解析)建议显式标记。
        assert!(refs.iter().any(|r| r.confidence >= 0.6));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn experience_save_load_and_search() {
        let root = std::env::temp_dir().join(format!("ah-evolve-exp-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");

        let experience = Experience {
            id: "exp1".to_string(),
            task: "list the workspace".to_string(),
            verdict: Verdict::Pass,
            score: 0.9,
            issues: vec![],
            saved_ms: 1234,
        };
        runtime.save_experience(&experience).expect("save");
        runtime
            .save_experience(&Experience {
                id: "exp2".to_string(),
                task: "quantum physics".to_string(),
                verdict: Verdict::Fail,
                score: 0.1,
                issues: vec!["unfinished".to_string()],
                saved_ms: 2345,
            })
            .expect("save2");

        let all = runtime.load_experiences().expect("load");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "exp1");
        assert_eq!(all[0].verdict, Verdict::Pass);

        // 检索:按任务标题匹配。
        let hits = runtime.search_experiences("workspace").expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "exp1");
        assert!(runtime.search_experiences("physics").expect("s2")[0].score < 0.5);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn experience_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-evolve-re-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let runtime = ctx
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        runtime
            .save_experience(&Experience {
                id: "e1".to_string(),
                task: "persist me".to_string(),
                verdict: Verdict::NeedsWork,
                score: 0.5,
                issues: vec![],
                saved_ms: 1,
            })
            .expect("save");
        drop(effects);

        // 重开:同一经验目录恢复。
        let reopened = build_ctx(&root);
        let runtime2 = reopened
            .0
            .service::<dyn EvolvingRuntime>(&EVOLVING)
            .expect("evolving");
        let all = runtime2.load_experiences().expect("load");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].task, "persist me");

        drop(reopened.1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
