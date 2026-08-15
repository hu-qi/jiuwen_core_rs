//! symphony seam:能力资产发现/检索/编排/执行(按 openjiuwen/symphony README 语义)。
//!
//! 能力 = 名称/描述/标签/可选工具;注册生成语义指纹;检索按标签与描述关键词;
//! 编排产出可解释执行路线;执行经工具或 subagent 真实运行。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 能力资产。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 可选绑定工具名(经工具注册表执行)。
    #[serde(default)]
    pub tool: Option<String>,
}

/// 能力指纹(语义画像)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityFingerprint {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    /// 输入语义提示(描述前 60 字)。
    pub input_hint: String,
    /// 输出语义提示(描述关键词)。
    pub output_hint: String,
}

/// 执行路线中的一步。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionStep {
    pub capability_id: String,
    pub name: String,
    pub detail: String,
}

/// 编排计划。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OrchestrationPlan {
    pub task: String,
    pub steps: Vec<ExecutionStep>,
    pub rationale: String,
}

/// symphony 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymphonyError(pub String);

impl core::fmt::Display for SymphonyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SymphonyError {}

/// symphony Seam(Service Definition):能力编排。
#[async_trait]
pub trait Symphony: Seam {
    /// 注册能力(真实持久化 + 生成指纹)。
    fn register_capability(&self, capability: Capability) -> Result<(), SymphonyError>;

    /// 能力指纹。
    fn fingerprint(&self, id: &str) -> Option<CapabilityFingerprint>;

    /// 按任务检索候选能力(标签/描述关键词匹配)。
    fn find_capabilities(&self, task: &str) -> Vec<Capability>;

    /// 编排:检索 → 排序 → 生成可解释执行路线。
    fn plan(&self, task: &str) -> Result<OrchestrationPlan, SymphonyError>;

    /// 执行路线(工具调用或 subagent 委派),返回聚合输出。
    async fn execute(&self, plan: &OrchestrationPlan, task: &str) -> Result<Value, SymphonyError>;
}
