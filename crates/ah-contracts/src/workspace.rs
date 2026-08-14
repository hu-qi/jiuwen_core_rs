//! workspace seam:工作区清单与目标跟踪(workspace-goal-manifest)。
//!
//! 对应 openjiuwen/harness 的 workspace/goal/manifest:一份真实的
//! workspace.json 描述工作区(名称 + 目标列表 + 目标状态机),增改即落盘。

use crate::seam::Seam;

/// 目标状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Pending,
    InProgress,
    Done,
    Abandoned,
}

/// 一个目标。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub status: GoalStatus,
    pub created_ms: u64,
    pub updated_ms: u64,
}

/// 工作区清单。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceManifest {
    pub name: String,
    pub description: String,
    pub goals: Vec<Goal>,
    pub updated_ms: u64,
}

/// workspace 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceError(pub String);

impl core::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkspaceError {}

/// workspace Seam(Service Definition):清单加载/保存与目标生命周期。
pub trait WorkspaceService: Seam {
    /// 加载清单;文件不存在时以默认清单初始化并落盘(真实 create)。
    fn load(&self) -> Result<WorkspaceManifest, WorkspaceError>;

    /// 保存清单(真实 JSON 落盘)。
    fn save(&self, manifest: &WorkspaceManifest) -> Result<(), WorkspaceError>;

    /// 新建目标(默认 Pending);id 冲突显式报错。
    fn create_goal(&self, id: &str, title: &str) -> Result<Goal, WorkspaceError>;

    /// 更新目标状态;目标不存在显式报错。
    fn update_goal_status(&self, id: &str, status: GoalStatus) -> Result<Goal, WorkspaceError>;

    /// 全部目标(按 id 排序)。
    fn goals(&self) -> Result<Vec<Goal>, WorkspaceError>;
}
