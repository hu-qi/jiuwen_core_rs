//! 技能创建信号检测(对齐 signal/skill_creation.py)。
//!
//! 纯逻辑:窗口指标收集(skill_tool 覆盖/有效任务工具调用)+ 首次/再次提示阈值检测 + 工具名归一化。

use serde_json::Value;

pub const SKILL_CREATION_SIGNAL_PROMPT_ELIGIBLE: &str = "prompt_eligible";
pub const SKILL_CREATION_SIGNAL_SKILL_TOOL_COVER: &str = "skill_tool_cover";

pub const FIRST_PROMPT_TOOL_ITERATION_THRESHOLD: usize = 6;
pub const FIRST_PROMPT_TOOL_CALL_THRESHOLD: usize = 10;
pub const REPROMPT_TOOL_ITERATION_THRESHOLD: usize = 2;
pub const REPROMPT_TOOL_CALL_THRESHOLD: usize = 4;

const EXCLUDED_TOOL_NAMES: [&str; 12] = [
    "ask_user",
    "skill_tool",
    "prepare_skill_evolution",
    "evolve_review_task",
    "evolve_skill_experiences",
    "spawn_member",
    "spawn_teammate",
    "spawn_human_agent",
    "spawn_bridge_agent",
    "spawn_external_cli",
    "send_message",
    "view_task",
];

const EXCLUDED_TOOL_KEYWORDS: [&str; 5] =
    ["follow_up", "followup", "heartbeat", "cron", "background"];

/// 归一化命名空间工具名(对齐 normalize_tool_name:取最后一段)。
pub fn normalize_tool_name(tool_name: &str) -> String {
    let tool = tool_name.trim();
    match tool.rsplit_once('.') {
        Some((_, base)) => base.to_string(),
        None => tool.to_string(),
    }
}

/// 是否有效任务工具(排除集合/关键词;对齐 is_effective_task_tool)。
pub fn is_effective_task_tool(tool_name: &str) -> bool {
    let tool = normalize_tool_name(tool_name);
    if tool.is_empty() {
        return false;
    }
    if EXCLUDED_TOOL_NAMES.contains(&tool.as_str()) {
        return false;
    }
    !EXCLUDED_TOOL_KEYWORDS.iter().any(|k| tool.contains(k))
}

/// 从 LLM response 提取 tool_calls(对齐 _iter_tool_calls;仅支持 dict 形状)。
pub fn iter_tool_calls(response: Option<&Value>) -> Vec<Value> {
    let Some(resp) = response else {
        return Vec::new();
    };
    resp.get("tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 工具调用名(function.name 优先;对齐 _tool_call_name)。
pub fn tool_call_name(tool_call: &Value) -> String {
    if let Some(function) = tool_call.get("function")
        && let Some(name) = function.get("name").and_then(|v| v.as_str())
        && !name.is_empty()
    {
        return name.to_string();
    }
    tool_call
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 工具调用 id(对齐 _tool_call_id)。
pub fn tool_call_id(tool_call: &Value) -> Option<String> {
    tool_call
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// response 是否含有效任务工具调用(对齐 _response_has_effective_tool_calls)。
pub fn response_has_effective_tool_calls(response: Option<&Value>) -> bool {
    iter_tool_calls(response)
        .iter()
        .any(|tc| is_effective_task_tool(&tool_call_name(tc)))
}

/// response 是否引用给定 tool_call_id 之一(对齐 _response_has_tool_call_id)。
pub fn response_has_tool_call_id(
    response: Option<&Value>,
    tool_call_ids: &std::collections::BTreeSet<String>,
) -> bool {
    iter_tool_calls(response)
        .iter()
        .any(|tc| match tool_call_id(tc) {
            Some(id) => tool_call_ids.contains(&id),
            None => false,
        })
}

/// 轨迹步骤视图(skill_creation 所需的步骤字段)。
#[derive(Debug, Clone, PartialEq)]
pub enum SkillStepView {
    /// 工具步骤:名称 / tool_call_id。
    Tool {
        tool_name: String,
        tool_call_id: Option<String>,
    },
    /// LLM 步骤:response(可能含 tool_calls)。
    Llm { response: Option<Value> },
}

/// 窗口指标(对齐 SkillCreationWindowMetrics)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillCreationWindowMetrics {
    pub total_raw_tool_calls: usize,
    pub window_effective_tool_calling_iterations: usize,
    pub window_effective_tool_calls: usize,
    pub total_effective_tool_calling_iterations: usize,
    pub total_effective_tool_calls: usize,
    pub skill_tool_used_in_window: bool,
}

/// 统计发起有效任务工具调用的 LLM 迭代数(对齐 count_tool_calling_iterations)。
pub fn count_tool_calling_iterations(steps: &[SkillStepView]) -> usize {
    steps
        .iter()
        .filter(|step| match step {
            SkillStepView::Llm { response } => response_has_effective_tool_calls(response.as_ref()),
            SkillStepView::Tool { .. } => false,
        })
        .count()
}

/// 窗口内新增有效工具调用迭代数(对齐 _count_new_effective_tool_calling_iterations)。
pub fn count_new_effective_tool_calling_iterations(
    steps: &[SkillStepView],
    effective_call_ids_after_watermark: &std::collections::BTreeSet<String>,
    new_tool_steps: &[&SkillStepView],
) -> usize {
    if new_tool_steps.is_empty() {
        return 0;
    }
    if !effective_call_ids_after_watermark.is_empty() {
        return steps
            .iter()
            .filter(|step| match step {
                SkillStepView::Llm { response } => {
                    response_has_tool_call_id(response.as_ref(), effective_call_ids_after_watermark)
                }
                SkillStepView::Tool { .. } => false,
            })
            .count();
    }
    // fallback:无 tool_call_id 关联时,按窗口内有效工具步骤计数
    new_tool_steps
        .iter()
        .filter(|step| match step {
            SkillStepView::Tool { tool_name, .. } => is_effective_task_tool(tool_name),
            SkillStepView::Llm { .. } => false,
        })
        .count()
}

/// 收集窗口指标(对齐 collect_metrics)。
pub fn collect_metrics(
    steps: &[SkillStepView],
    raw_tool_call_watermark: usize,
) -> SkillCreationWindowMetrics {
    if steps.is_empty() {
        return SkillCreationWindowMetrics {
            total_raw_tool_calls: 0,
            window_effective_tool_calling_iterations: 0,
            window_effective_tool_calls: 0,
            total_effective_tool_calling_iterations: 0,
            total_effective_tool_calls: 0,
            skill_tool_used_in_window: false,
        };
    }
    let tool_steps: Vec<&SkillStepView> = steps
        .iter()
        .filter(|s| matches!(s, SkillStepView::Tool { .. }))
        .collect();
    let total_raw_tool_calls = tool_steps.len();
    let watermark = raw_tool_call_watermark;
    let window_tool_steps: Vec<&SkillStepView> =
        tool_steps.iter().skip(watermark).copied().collect();

    let mut effective_call_ids_after_watermark: std::collections::BTreeSet<String> =
        Default::default();
    let mut window_effective_tool_calls = 0usize;
    let mut total_effective_tool_calls = 0usize;
    let mut skill_tool_used_in_window = false;
    for (index, step) in tool_steps.iter().enumerate() {
        let SkillStepView::Tool {
            tool_name,
            tool_call_id,
        } = step
        else {
            continue;
        };
        if index >= watermark && normalize_tool_name(tool_name) == "skill_tool" {
            skill_tool_used_in_window = true;
        }
        if !is_effective_task_tool(tool_name) {
            continue;
        }
        total_effective_tool_calls += 1;
        if index >= watermark {
            window_effective_tool_calls += 1;
            if let Some(id) = tool_call_id {
                effective_call_ids_after_watermark.insert(id.clone());
            }
        }
    }

    let total_effective_tool_calling_iterations = count_tool_calling_iterations(steps);
    let window_effective_tool_calling_iterations = count_new_effective_tool_calling_iterations(
        steps,
        &effective_call_ids_after_watermark,
        &window_tool_steps,
    );

    SkillCreationWindowMetrics {
        total_raw_tool_calls,
        window_effective_tool_calling_iterations,
        window_effective_tool_calls,
        total_effective_tool_calling_iterations,
        total_effective_tool_calls,
        skill_tool_used_in_window,
    }
}

/// 检测出的技能创建信号(对齐 SkillCreationSignal)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillCreationSignal {
    pub signal_type: String,
    pub metrics: SkillCreationWindowMetrics,
    pub reason: String,
}

/// 信号检测器(对齐 SkillCreationSignalDetector)。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkillCreationSignalDetector;

impl SkillCreationSignalDetector {
    pub fn detect(
        &self,
        steps: &[SkillStepView],
        raw_tool_call_watermark: usize,
        prompted_snapshot: Option<(usize, usize)>,
        metrics: Option<&SkillCreationWindowMetrics>,
    ) -> Vec<SkillCreationSignal> {
        if steps.is_empty() {
            return Vec::new();
        }
        let current = match metrics {
            Some(m) => m.clone(),
            None => collect_metrics(steps, raw_tool_call_watermark),
        };

        if current.skill_tool_used_in_window {
            return vec![SkillCreationSignal {
                signal_type: SKILL_CREATION_SIGNAL_SKILL_TOOL_COVER.to_string(),
                metrics: current,
                reason: "skill_tool_used".to_string(),
            }];
        }

        let Some((prompted_iterations, prompted_calls)) = prompted_snapshot else {
            if current.window_effective_tool_calling_iterations
                >= FIRST_PROMPT_TOOL_ITERATION_THRESHOLD
                || current.window_effective_tool_calls >= FIRST_PROMPT_TOOL_CALL_THRESHOLD
            {
                return vec![SkillCreationSignal {
                    signal_type: SKILL_CREATION_SIGNAL_PROMPT_ELIGIBLE.to_string(),
                    metrics: current,
                    reason: "first_prompt_threshold".to_string(),
                }];
            }
            return Vec::new();
        };

        let new_iterations = current
            .total_effective_tool_calling_iterations
            .saturating_sub(prompted_iterations);
        let new_calls = current
            .total_effective_tool_calls
            .saturating_sub(prompted_calls);
        if new_iterations >= REPROMPT_TOOL_ITERATION_THRESHOLD
            || new_calls >= REPROMPT_TOOL_CALL_THRESHOLD
        {
            return vec![SkillCreationSignal {
                signal_type: SKILL_CREATION_SIGNAL_PROMPT_ELIGIBLE.to_string(),
                metrics: current,
                reason: "reprompt_threshold".to_string(),
            }];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_step(name: &str, id: Option<&str>) -> SkillStepView {
        SkillStepView::Tool {
            tool_name: name.to_string(),
            tool_call_id: id.map(|s| s.to_string()),
        }
    }

    fn llm_step(tool_calls: Value) -> SkillStepView {
        SkillStepView::Llm {
            response: Some(json!({ "tool_calls": tool_calls })),
        }
    }

    #[test]
    fn normalize_tool_name_strips_namespace() {
        assert_eq!(normalize_tool_name("mcp.foo.bash"), "bash");
        assert_eq!(normalize_tool_name("  code  "), "code");
        assert_eq!(normalize_tool_name(""), "");
    }

    #[test]
    fn effective_task_tool_classification() {
        assert!(is_effective_task_tool("bash"));
        assert!(!is_effective_task_tool("ask_user"));
        assert!(!is_effective_task_tool("send_message"));
        assert!(!is_effective_task_tool("spawn_member"));
        assert!(!is_effective_task_tool("tool_follow_up"));
        assert!(!is_effective_task_tool("heartbeat_check"));
        assert!(!is_effective_task_tool(""));
        assert!(!is_effective_task_tool("skill_tool"));
    }

    #[test]
    fn tool_call_name_and_id_extraction() {
        let tc = json!({"id": "call_1", "function": {"name": "bash", "arguments": "{}"}});
        assert_eq!(tool_call_name(&tc), "bash");
        assert_eq!(tool_call_id(&tc).as_deref(), Some("call_1"));
        let tc2 = json!({"id": "call_2", "name": "code"});
        assert_eq!(tool_call_name(&tc2), "code");
        assert_eq!(tool_call_id(&tc2).as_deref(), Some("call_2"));
    }

    #[test]
    fn count_iterations_with_effective_calls() {
        let steps = vec![
            llm_step(json!([{"function": {"name": "bash"}}])),
            llm_step(json!([{"function": {"name": "ask_user"}}])),
            llm_step(json!([])),
        ];
        assert_eq!(count_tool_calling_iterations(&steps), 1);
    }

    #[test]
    fn collect_metrics_window_counts() {
        // 2 原始工具步骤,watermark=1 只统计窗口内
        let steps = vec![tool_step("bash", Some("c1")), tool_step("bash", Some("c2"))];
        let m = collect_metrics(&steps, 1);
        assert_eq!(m.total_raw_tool_calls, 2);
        assert_eq!(m.window_effective_tool_calls, 1);
        assert_eq!(m.total_effective_tool_calls, 2);
        assert!(!m.skill_tool_used_in_window);
    }

    #[test]
    fn skill_tool_cover_wins() {
        let detector = SkillCreationSignalDetector;
        let steps = vec![tool_step("skill_tool", Some("c1"))];
        let signals = detector.detect(&steps, 0, None, None);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, "skill_tool_cover");
        assert_eq!(signals[0].reason, "skill_tool_used");
    }

    #[test]
    fn first_prompt_thresholds() {
        let detector = SkillCreationSignalDetector;
        let mut steps = Vec::new();
        for i in 0..10 {
            steps.push(tool_step("bash", Some(&format!("c{i}"))));
        }
        let signals = detector.detect(&steps, 0, None, None);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, "prompt_eligible");
        assert_eq!(signals[0].reason, "first_prompt_threshold");
        // 少量工具步骤 → 无信号
        let few = vec![tool_step("bash", Some("c1"))];
        assert!(detector.detect(&few, 0, None, None).is_empty());
    }

    #[test]
    fn reprompt_threshold_uses_delta() {
        let detector = SkillCreationSignalDetector;
        let mut steps = Vec::new();
        for i in 0..6 {
            steps.push(tool_step("bash", Some(&format!("c{i}"))));
        }
        // 快照已记录 6 次迭代;无新增 → 无信号
        let no_new = detector.detect(&steps, 0, Some((6, 6)), None);
        assert!(no_new.is_empty());
        // 新增 4 次调用(超过 REPROMPT_TOOL_CALL_THRESHOLD=4,等于不触发?4>=4 触发)
        let mut steps2 = Vec::new();
        for i in 0..10 {
            steps2.push(tool_step("bash", Some(&format!("c{i}"))));
        }
        let reprompt = detector.detect(&steps2, 0, Some((6, 6)), None);
        assert_eq!(reprompt.len(), 1);
        assert_eq!(reprompt[0].reason, "reprompt_threshold");
    }
}
