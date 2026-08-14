//! auto_harness seam:自动化改进周期(assess→plan→implement→verify→commit→publish)。
//!
//! 每阶段真实执行:assess 用 git 状态、plan 用 rsi 数据集、implement 用 subagent 真实委派、
//! verify 用 ci 门禁、commit 用 git 提交、publish 用 git 分支。任一阶段失败即停。

use async_trait::async_trait;
use std::path::PathBuf;

use crate::seam::Seam;

/// 周期配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutoHarnessConfig {
    pub task: String,
    pub workspace: PathBuf,
    /// 门禁命令列表(argv 形式,如 [["cargo","clippy","--workspace"]]);空 = 跳过 verify 门禁。
    pub gates: Vec<Vec<String>>,
}

/// 阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageKind {
    Assess,
    Plan,
    Implement,
    Verify,
    Commit,
    Publish,
}

/// 单阶段结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StageResult {
    pub stage: StageKind,
    pub ok: bool,
    pub detail: String,
    /// commit 阶段产生的提交哈希(如有)。
    pub commit: Option<String>,
}

/// 周期结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CycleResult {
    pub stages: Vec<StageResult>,
    /// 失败阶段(有则停在它)。
    pub failed_at: Option<StageKind>,
}

/// auto_harness 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoHarnessError(pub String);

impl core::fmt::Display for AutoHarnessError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AutoHarnessError {}

/// auto_harness Seam(Service Definition):自动化改进周期。
#[async_trait]
pub trait AutoHarness: Seam {
    /// 执行完整周期;任一阶段失败即停,返回已执行阶段与失败点。
    async fn run_cycle(&self, config: AutoHarnessConfig) -> Result<CycleResult, AutoHarnessError>;
}
