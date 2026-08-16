//! signals seam:演化信号(对齐 openjiuwen/rsi/team_skill_optimizer/signals.py + agent_evolving/signal/base.py)。
//!
//! 把分析器问题(issue)映射为演化信号:
//! - 信号类型归因:attribution.target_ref + category 关键词 → routing_policy / handoff_protocol /
//!   shared_context_contract / final_answer_verification / stop_condition / team_coordination;
//! - 问题归一化:severity ∈ {low,medium,high}(非法归 medium)+ issue_type/description/affected_role;
//! - 摘录与用户查询构建(供 LLM 演化步骤使用)。
//!
//! 契约零实现:归因算法由插件提供(如 ah-plugins-signals)。

use serde_json::{Map, Value};

use crate::seam::Seam;

/// 一条演化信号(对齐 EvolutionSignal)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvolutionSignal {
    pub signal_type: String,
    pub section: String,
    pub excerpt: String,
    pub skill_name: Option<String>,
    pub context: Option<Map<String, Value>>,
}

/// signals 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalsError(pub String);

impl core::fmt::Display for SignalsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignalsError {}

/// 演化信号 Seam(Service Definition):问题 → 信号映射。
pub trait Signals: Seam {
    /// 问题类型归因(attribution.target_ref + category 关键词)。
    fn issue_type(&self, issue: &Value) -> String;

    /// 归一化问题(severity 规整 + type/description/affected_role)。
    fn normalize_trajectory_issue(&self, issue: &Value) -> Value;

    /// 问题描述(summary/recommendation + attribution 字段拼接;空则回退摘录)。
    fn issue_description(&self, issue: &Value) -> String;

    /// 受影响角色(affected_components[0] → evidence.affected_component → 空)。
    fn affected_role(&self, issue: &Value) -> String;

    /// 摘录(summary/recommendation/description 首个非空,截断 1000;回退 issue_id)。
    fn issue_excerpt(&self, issue: &Value) -> String;

    /// 问题 id 列表(issue_id/id,缺省 team_skill_issue_{index:03d},1 基)。
    fn issue_ids(&self, issues: &[Value]) -> Vec<String>;

    /// 用户查询(逐条 recommendation 前缀 '- ';无则用摘录)。
    fn build_user_query(&self, issues: &[Value]) -> String;

    /// 批量构建演化信号(每条问题一个 trajectory_issue 信号,上下文携带归一化问题等)。
    fn build_signals(
        &self,
        issues: &[Value],
        skill_name: &str,
        skill_content: &str,
        eval_ref_path: &str,
        analysis_result_path: &str,
    ) -> Vec<EvolutionSignal>;
}
