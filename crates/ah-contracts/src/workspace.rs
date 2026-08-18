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
// ---------------------------------------------------------------------------
// Workspace 目录模型(对齐 harness/workspace/workspace.py)
// ---------------------------------------------------------------------------

/// 工作区目录节点名(对齐 WorkspaceNode 枚举)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceNode {
    AgentMd,
    SoulMd,
    HeartbeatMd,
    IdentityMd,
    UserMd,
    Memory,
    CodingMemory,
    Todo,
    Messages,
    Skills,
    Agents,
    MemoryMd,
    DailyMemory,
    TeamLinks,
    WorktreeLinks,
}

impl WorkspaceNode {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceNode::AgentMd => "AGENT.md",
            WorkspaceNode::SoulMd => "SOUL.md",
            WorkspaceNode::HeartbeatMd => "HEARTBEAT.md",
            WorkspaceNode::IdentityMd => "IDENTITY.md",
            WorkspaceNode::UserMd => "USER.md",
            WorkspaceNode::Memory => "memory",
            WorkspaceNode::CodingMemory => "coding_memory",
            WorkspaceNode::Todo => "todo",
            WorkspaceNode::Messages => "messages",
            WorkspaceNode::Skills => "skills",
            WorkspaceNode::Agents => "agents",
            WorkspaceNode::MemoryMd => "MEMORY.md",
            WorkspaceNode::DailyMemory => "daily_memory",
            WorkspaceNode::TeamLinks => ".team",
            WorkspaceNode::WorktreeLinks => ".worktree",
        }
    }
}

/// 目录节点定义(对齐 DirectoryNode;name/path/description/children/is_file)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectoryNode {
    pub name: String,
    pub path: String,
    pub description: String,
    pub is_file: bool,
    pub children: Vec<DirectoryNode>,
}

impl DirectoryNode {
    pub fn new(
        name: impl Into<String>,
        path: impl Into<String>,
        description: impl Into<String>,
        is_file: bool,
    ) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            description: description.into(),
            is_file,
            children: Vec::new(),
        }
    }

    pub fn with_children(mut self, children: Vec<DirectoryNode>) -> Self {
        self.children = children;
        self
    }
}

/// 工作区默认目录 schema(对齐 DEFAULT_WORKSPACE_SCHEMA 的核心节点)。
pub fn default_workspace_schema() -> Vec<DirectoryNode> {
    vec![
        DirectoryNode::new("AGENT.md", "AGENT.md", "基础配置和能力", true),
        DirectoryNode::new("SOUL.md", "SOUL.md", "人格、性格和价值观", true),
        DirectoryNode::new("HEARTBEAT.md", "HEARTBEAT.md", "心跳日志和状态记录", true),
        DirectoryNode::new("IDENTITY.md", "IDENTITY.md", "身份凭证和权限", true),
        DirectoryNode::new("USER.md", "USER.md", "用户数据目录", true),
        DirectoryNode::new("memory", "memory", "记忆核心模块", false).with_children(vec![
            DirectoryNode::new("MEMORY.md", "MEMORY.md", "长期记忆索引和摘要", true),
            DirectoryNode::new("daily_memory", "daily_memory", "每日结构化记忆", false),
        ]),
        DirectoryNode::new(
            "coding_memory",
            "coding_memory",
            "Coding Agent 记忆模块",
            false,
        )
        .with_children(vec![DirectoryNode::new(
            "MEMORY.md",
            "MEMORY.md",
            "Coding 记忆索引",
            true,
        )]),
        DirectoryNode::new("todo", "todo", "待办事项目录", false),
        DirectoryNode::new("messages", "messages", "消息历史目录", false),
        DirectoryNode::new("skills", "skills", "技能库目录", false),
        DirectoryNode::new("agents", "agents", "子智能体嵌套目录", false),
    ]
}

/// 在目录节点树中按 name 递归查找 path(对齐 get_directory 的 find_in_nodes)。
pub fn find_directory_path<'a>(nodes: &'a [DirectoryNode], name: &str) -> Option<&'a str> {
    for node in nodes {
        if node.name == name {
            return Some(&node.path);
        }
        if let Some(p) = find_directory_path(&node.children, name) {
            return Some(p);
        }
    }
    None
}

/// 工作区目录解析(对齐 Workspace.get_directory:先查显式目录,再回退默认 schema)。
pub fn resolve_directory(directories: &[DirectoryNode], name: &str) -> Option<String> {
    if let Some(p) = find_directory_path(directories, name) {
        return Some(p.to_string());
    }
    find_directory_path(&default_workspace_schema(), name).map(|p| p.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_node_str_values() {
        assert_eq!(WorkspaceNode::Memory.as_str(), "memory");
        assert_eq!(WorkspaceNode::MemoryMd.as_str(), "MEMORY.md");
        assert_eq!(WorkspaceNode::AgentMd.as_str(), "AGENT.md");
        assert_eq!(WorkspaceNode::DailyMemory.as_str(), "daily_memory");
    }

    #[test]
    fn default_schema_has_core_nodes() {
        let schema = default_workspace_schema();
        assert!(schema.iter().any(|n| n.name == "memory"));
        assert!(schema.iter().any(|n| n.name == "AGENT.md"));
        assert!(schema.iter().any(|n| n.name == "coding_memory"));
        // memory has children
        let mem = schema.iter().find(|n| n.name == "memory").unwrap();
        assert!(mem.children.iter().any(|c| c.name == "MEMORY.md"));
    }

    #[test]
    fn find_directory_path_recursive() {
        let schema = default_workspace_schema();
        assert_eq!(find_directory_path(&schema, "memory"), Some("memory"));
        assert_eq!(find_directory_path(&schema, "MEMORY.md"), Some("MEMORY.md"));
        assert_eq!(
            find_directory_path(&schema, "daily_memory"),
            Some("daily_memory")
        );
        assert_eq!(find_directory_path(&schema, "nope"), None);
    }

    #[test]
    fn resolve_directory_falls_back_to_default() {
        // explicit empty -> default fallback
        assert_eq!(resolve_directory(&[], "skills"), Some("skills".to_string()));
        assert_eq!(resolve_directory(&[], "agents"), Some("agents".to_string()));
        // explicit overrides default
        let explicit = vec![DirectoryNode::new("skills", "custom_skills", "x", false)];
        assert_eq!(
            resolve_directory(&explicit, "skills"),
            Some("custom_skills".to_string())
        );
        // missing from both -> None
        assert_eq!(resolve_directory(&[], "ghost"), None);
    }
}
