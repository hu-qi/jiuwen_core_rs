//! 团队域信号助手(对齐 signal/team.py)。
//!
//! 纯逻辑:团队信号类型/意图/轨迹问题结构 + 轨迹摘要构建 + 显式请求/被动轨迹信号 + 上下文读取。

use crate::protocols::{TRAJECTORY_ISSUE_SIGNAL, USER_INTENT_SIGNAL};
use crate::signal::{EvolutionSignal, make_evolution_signal};
use serde_json::Value;

/// 团队域信号类型(对齐 TeamSignalType;USER_REQUEST 为兼容别名)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamSignalType {
    UserIntent,
    UserRequest,
    TrajectoryIssue,
}

impl TeamSignalType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserIntent => USER_INTENT_SIGNAL,
            Self::UserRequest => "user_request",
            Self::TrajectoryIssue => TRAJECTORY_ISSUE_SIGNAL,
        }
    }
}

/// 解析出的用户改进意图(对齐 UserIntent)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UserIntent {
    pub is_improvement: bool,
    pub intent: String,
}

/// 归一化的轨迹问题(对齐 TrajectoryIssue)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryIssue {
    pub issue_type: String,
    pub description: String,
    #[serde(default)]
    pub affected_role: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_severity() -> String {
    "medium".to_string()
}

/// 轨迹步骤视图(team 摘要构建所需的步骤字段;对齐 trajectory_steps 的 tool/llm 明细)。
#[derive(Debug, Clone, PartialEq)]
pub enum TeamStepView {
    /// 工具步骤:名称/参数/结果。
    Tool {
        tool_name: String,
        call_args: Option<String>,
        call_result: Option<String>,
    },
    /// LLM 步骤:响应文本(空视为无响应)。
    Llm { response: Option<String> },
}

const KEY_TOOLS: [&str; 5] = [
    "spawn_member",
    "create_task",
    "build_team",
    "view_task",
    "send_message",
];

const TOOL_BUDGET: usize = 20000;
const LLM_BUDGET: usize = 10000;

/// 团队轨迹摘要(对齐 build_team_trajectory_summary:关键工具更高详情 + 预算截断)。
pub fn build_team_trajectory_summary(steps: &[TeamStepView]) -> String {
    let mut tool_lines: Vec<String> = Vec::new();
    let mut llm_lines: Vec<String> = Vec::new();
    let mut llm_count = 0usize;
    let mut tool_count = 0usize;

    for step in steps {
        match step {
            TeamStepView::Tool {
                tool_name,
                call_args,
                call_result,
            } => {
                tool_count += 1;
                let is_key = KEY_TOOLS.contains(&tool_name.as_str());
                let args_limit = if is_key { 500 } else { 150 };
                let result_limit = if is_key { 500 } else { 200 };
                let args: String = call_args
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .take(args_limit)
                    .collect();
                let result: String = call_result
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .take(result_limit)
                    .collect();
                tool_lines.push(format!("[Tool:{tool_name}] args={args} result={result}"));
            }
            TeamStepView::Llm { response } => {
                llm_count += 1;
                if let Some(resp) = response
                    && !resp.is_empty()
                {
                    let clipped: String = resp.chars().take(300).collect();
                    llm_lines.push(format!("[LLM] {clipped}"));
                }
            }
        }
    }

    let mut tool_section = tool_lines.join("\n");
    if tool_section.chars().count() > TOOL_BUDGET {
        let clipped: String = tool_section.chars().take(TOOL_BUDGET).collect();
        tool_section = format!("{clipped}\n... (tool section truncated)");
    }

    let mut llm_section = llm_lines.join("\n");
    if llm_section.chars().count() > LLM_BUDGET {
        let clipped: String = llm_section.chars().take(LLM_BUDGET).collect();
        llm_section = format!("{clipped}\n... (LLM section truncated)");
    }

    format!(
        "### Tool Calls ({tool_count})\n{tool_section}\n\n### LLM Responses ({llm_count})\n{llm_section}"
    )
}

const TEAM_TRAJECTORY_ISSUES_KEY: &str = "trajectory_issues";
const TEAM_SKILL_CONTENT_KEY: &str = "skill_content";

/// 显式请求信号(对齐 make_team_user_intent_signal)。
pub fn make_team_user_intent_signal(skill_name: &str, user_intent: &str) -> EvolutionSignal {
    make_evolution_signal(
        TeamSignalType::UserIntent.as_str(),
        "Instructions",
        user_intent,
        None,
        Some(skill_name),
        Some("explicit_request"),
        None,
    )
}

/// 被动轨迹信号(对齐 make_team_trajectory_signal)。
pub fn make_team_trajectory_signal(
    skill_name: &str,
    skill_content: &str,
    trajectory_issues: &[Value],
) -> EvolutionSignal {
    let mut context = std::collections::BTreeMap::new();
    context.insert(
        TEAM_TRAJECTORY_ISSUES_KEY.to_string(),
        Value::Array(trajectory_issues.to_vec()),
    );
    context.insert(
        TEAM_SKILL_CONTENT_KEY.to_string(),
        Value::String(skill_content.to_string()),
    );
    make_evolution_signal(
        TeamSignalType::TrajectoryIssue.as_str(),
        "",
        "Detected team skill trajectory issues requiring evolution.",
        None,
        Some(skill_name),
        Some("passive_trajectory"),
        Some(context),
    )
}

/// 读取归一化轨迹问题(对齐 get_team_trajectory_issues)。
pub fn get_team_trajectory_issues(signal: &EvolutionSignal) -> Vec<Value> {
    let context = signal.context.as_ref();
    let Some(issues) = context.and_then(|c| c.get(TEAM_TRAJECTORY_ISSUES_KEY)) else {
        return Vec::new();
    };
    match issues {
        Value::Array(items) => items.iter().filter(|v| v.is_object()).cloned().collect(),
        _ => Vec::new(),
    }
}

/// 读取关联团队技能内容(对齐 get_team_signal_skill_content)。
pub fn get_team_signal_skill_content(signal: &EvolutionSignal) -> Option<String> {
    let context = signal.context.as_ref()?;
    let content = context.get(TEAM_SKILL_CONTENT_KEY)?;
    match content {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn team_signal_type_values() {
        assert_eq!(TeamSignalType::UserIntent.as_str(), "user_intent");
        assert_eq!(TeamSignalType::UserRequest.as_str(), "user_request");
        assert_eq!(TeamSignalType::TrajectoryIssue.as_str(), "trajectory_issue");
    }

    #[test]
    fn trajectory_summary_formats_and_counts() {
        let steps = vec![
            TeamStepView::Tool {
                tool_name: "spawn_member".to_string(),
                call_args: Some("args".to_string()),
                call_result: Some("result".to_string()),
            },
            TeamStepView::Llm {
                response: Some("hello".to_string()),
            },
            TeamStepView::Llm { response: None },
        ];
        let summary = build_team_trajectory_summary(&steps);
        assert!(summary.contains("### Tool Calls (1)"));
        assert!(summary.contains("### LLM Responses (2)"));
        assert!(summary.contains("[Tool:spawn_member] args=args result=result"));
        assert!(summary.contains("[LLM] hello"));
    }

    #[test]
    fn key_tool_gets_longer_budget() {
        let steps = vec![
            TeamStepView::Tool {
                tool_name: "spawn_member".to_string(),
                call_args: Some("x".repeat(600)),
                call_result: None,
            },
            TeamStepView::Tool {
                tool_name: "ls".to_string(),
                call_args: Some("y".repeat(200)),
                call_result: None,
            },
        ];
        let summary = build_team_trajectory_summary(&steps);
        // key tool args 截断到 500,普通工具截断到 150
        assert!(summary.contains(&"x".repeat(500)));
        assert!(!summary.contains(&"y".repeat(151)));
        assert!(summary.contains(&"y".repeat(150)));
    }

    #[test]
    fn budget_truncation_appends_marker() {
        let mut steps = Vec::new();
        for _ in 0..60 {
            steps.push(TeamStepView::Tool {
                tool_name: "ls".to_string(),
                call_args: Some("a".repeat(200)),
                call_result: Some("b".repeat(200)),
            });
        }
        let summary = build_team_trajectory_summary(&steps);
        assert!(summary.contains("... (tool section truncated)"));
    }

    #[test]
    fn user_intent_signal_shape() {
        let s = make_team_user_intent_signal("sk", "please improve");
        assert_eq!(s.signal_type, "user_intent");
        assert_eq!(s.section, "Instructions");
        assert_eq!(s.excerpt, "please improve");
        assert_eq!(
            crate::signal::get_signal_source(&s).as_deref(),
            Some("explicit_request")
        );
    }

    #[test]
    fn trajectory_signal_context_roundtrip() {
        let issues = vec![
            json!({"issue_type": "x", "description": "d"}),
            json!("skip-me"),
        ];
        let s = make_team_trajectory_signal("sk", "content", &issues);
        assert_eq!(s.signal_type, "trajectory_issue");
        assert_eq!(
            crate::signal::get_signal_source(&s).as_deref(),
            Some("passive_trajectory"),
        );
        let read = get_team_trajectory_issues(&s);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0]["issue_type"], "x");
        assert_eq!(
            get_team_signal_skill_content(&s).as_deref(),
            Some("content"),
        );
    }

    #[test]
    fn empty_context_reads_default() {
        let empty = make_evolution_signal("x", "y", "z", None, None, None, None);
        assert!(get_team_trajectory_issues(&empty).is_empty());
        assert_eq!(get_team_signal_skill_content(&empty), None);
    }
}
