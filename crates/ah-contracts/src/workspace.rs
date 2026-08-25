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
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_file: bool,
    #[serde(default)]
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
    workspace_schema("cn")
}

/// 按语言返回工作区默认 schema(对齐 `get_workspace_schema`;cn/en 两套)。
pub fn workspace_schema(language: &str) -> Vec<DirectoryNode> {
    if language == "en" {
        vec![
            DirectoryNode::new(
                "AGENT.md",
                "AGENT.md",
                "Basic agent configuration and capabilities",
                true,
            ),
            DirectoryNode::new(
                "SOUL.md",
                "SOUL.md",
                "Agent personality, character, values, and behavioral guidelines",
                true,
            ),
            DirectoryNode::new(
                "HEARTBEAT.md",
                "HEARTBEAT.md",
                "Heartbeat log / status recording",
                true,
            ),
            DirectoryNode::new(
                "IDENTITY.md",
                "IDENTITY.md",
                "Identity credentials, unique identifier, and permission information",
                true,
            ),
            DirectoryNode::new("USER.md", "USER.md", "User data directory", true),
            DirectoryNode::new("memory", "memory", "Memory core module", false).with_children(
                vec![
                    DirectoryNode::new(
                        "MEMORY.md",
                        "MEMORY.md",
                        "Memory overview, index, and important memory summaries",
                        true,
                    ),
                    DirectoryNode::new(
                        "daily_memory",
                        "daily_memory",
                        "Daily structured memory",
                        false,
                    ),
                ],
            ),
            DirectoryNode::new("todo", "todo", "Todo items", false),
            DirectoryNode::new("messages", "messages", "Message history module", false),
            DirectoryNode::new("skills", "skills", "Skills library directory", false),
            DirectoryNode::new("agents", "agents", "Sub-agent nesting directory", false),
            DirectoryNode::new(
                "context",
                "context",
                "context offload and session memory file",
                false,
            )
            .with_children(vec![DirectoryNode::new(
                "session_memory.md",
                "session_memory.md",
                "session memory模版",
                true,
            )]),
        ]
    } else {
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
            DirectoryNode::new(
                "context",
                "context",
                "上下文offload以及session memory目录",
                false,
            )
            .with_children(vec![DirectoryNode::new(
                "session_memory.md",
                "session_memory.md",
                "session memory模版",
                true,
            )]),
        ]
    }
}

/// 校验单个目录节点(对齐 `_validate_directory_node`):非 dict/name 非空字符串/
/// name 含路径分隔符/path·description 非字符串/is_file·default_content 非
/// 对应类型/children 非列表,均显式报错。
pub fn validate_directory_node(node: &serde_json::Value) -> Result<(), WorkspaceError> {
    let Some(obj) = node.as_object() else {
        return Err(WorkspaceError(
            "Each directory entry must be a dict.".to_string(),
        ));
    };
    let name = obj.get("name");
    match name {
        Some(serde_json::Value::String(n)) if !n.is_empty() => {}
        _ => {
            return Err(WorkspaceError(
                "Directory `name` must be a non-empty string.".to_string(),
            ));
        }
    }
    let name_str = name.unwrap().as_str().unwrap();
    if name_str.contains('/') || name_str.contains('\\') {
        return Err(WorkspaceError(format!(
            "Directory `name` must not contain path separators: {name_str:?}"
        )));
    }
    if let Some(path) = obj.get("path")
        && !path.is_string()
    {
        return Err(WorkspaceError(
            "Directory `path` must be a string when provided.".to_string(),
        ));
    }
    if let Some(description) = obj.get("description")
        && !description.is_string()
    {
        return Err(WorkspaceError(
            "Directory `description` must be a string when provided.".to_string(),
        ));
    }
    if let Some(is_file) = obj.get("is_file")
        && !is_file.is_boolean()
    {
        return Err(WorkspaceError(
            "`is_file` must be a bool when provided.".to_string(),
        ));
    }
    if let Some(default_content) = obj.get("default_content")
        && !default_content.is_string()
    {
        return Err(WorkspaceError(
            "`default_content` must be a string when provided.".to_string(),
        ));
    }
    if let Some(children) = obj.get("children") {
        let Some(children) = children.as_array() else {
            return Err(WorkspaceError(
                "Directory `children` must be a list when provided.".to_string(),
            ));
        };
        for child in children {
            validate_directory_node(child)?;
        }
    }
    Ok(())
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

/// 顶层节点完整路径(对齐 Workspace.get_node_path:仅查顶层,root_path 拼接)。
pub fn node_full_path(
    directories: &[DirectoryNode],
    root_path: &str,
    name: &str,
) -> Option<String> {
    let relative = directories
        .iter()
        .find(|n| n.name == name)
        .map(|n| n.path.clone())
        .unwrap_or_else(|| name.to_string());
    let root = if root_path.is_empty() { "." } else { root_path };
    Some(format!("{}/{}", root.trim_end_matches('/'), relative))
}

/// 顶层节点添加/替换(对齐 Workspace.set_directory):同名替换,否则追加。
pub fn set_directory(directories: &mut Vec<DirectoryNode>, node: DirectoryNode) {
    let name = node.name.clone();
    for existing in directories.iter_mut() {
        if existing.name == name {
            *existing = node;
            return;
        }
    }
    directories.push(node);
}

/// 目录构建路径安全校验(对齐 directory_builder.py `DirectoryBuilder._is_safe_path`):
/// 绝对路径/盘符/`..` 越级均不安全。
pub fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    // Windows 盘符(C:\)。
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }
    if path.starts_with("\\\\") {
        return false;
    }
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|x| !x.is_empty() && *x != ".")
        .collect();
    !parts.contains(&"..")
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

    #[test]
    fn workspace_schema_language_variants() {
        let cn = workspace_schema("cn");
        let en = workspace_schema("en");
        // 两套语言都有 core 节点。
        assert!(cn.iter().any(|n| n.name == "context"));
        assert!(en.iter().any(|n| n.name == "context"));
        // 语言差异:cn 有 coding_memory,en 没有。
        assert!(cn.iter().any(|n| n.name == "coding_memory"));
        assert!(!en.iter().any(|n| n.name == "coding_memory"));
        // en 描述为英文。
        let en_agent = en.iter().find(|n| n.name == "AGENT.md").unwrap();
        assert!(en_agent.description.starts_with("Basic agent"));
        // 未知语言回退 cn。
        assert!(
            workspace_schema("fr")
                .iter()
                .any(|n| n.name == "coding_memory")
        );
    }

    #[test]
    fn validate_directory_node_rejects_bad_shapes() {
        use serde_json::json;
        // 合法节点通过。
        assert!(
            validate_directory_node(&json!({
                "name": "skills", "path": "skills", "is_file": false, "children": []
            }))
            .is_ok()
        );
        // 非 dict 报错。
        assert!(validate_directory_node(&json!("skills")).is_err());
        // name 缺失/空/含分隔符。
        assert!(validate_directory_node(&json!({"path": "x"})).is_err());
        assert!(validate_directory_node(&json!({"name": ""})).is_err());
        assert!(validate_directory_node(&json!({"name": "a/b"})).is_err());
        // 字段类型错误。
        assert!(validate_directory_node(&json!({"name": "x", "path": 1})).is_err());
        assert!(validate_directory_node(&json!({"name": "x", "is_file": "yes"})).is_err());
        assert!(validate_directory_node(&json!({"name": "x", "children": {}})).is_err());
        // 嵌套子节点递归校验。
        assert!(
            validate_directory_node(&json!({
                "name": "mem", "children": [{"name": "bad/name"}]
            }))
            .is_err()
        );
    }

    #[test]
    fn node_full_path_and_set_directory() {
        let mut dirs = vec![DirectoryNode::new("skills", "skills", "x", false)];
        // 顶层路径拼接。
        assert_eq!(
            node_full_path(&dirs, "/root", "skills"),
            Some("/root/skills".to_string())
        );
        assert_eq!(
            node_full_path(&dirs, "/root/", "skills"),
            Some("/root/skills".to_string())
        );
        // 未知顶层 → name 自身。
        assert_eq!(
            node_full_path(&dirs, "/root", "ghost"),
            Some("/root/ghost".to_string())
        );
        // set_directory:替换同名。
        set_directory(
            &mut dirs,
            DirectoryNode::new("skills", "custom", "y", false),
        );
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].path, "custom");
        // 追加新名。
        set_directory(
            &mut dirs,
            DirectoryNode::new("agents", "agents", "z", false),
        );
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    fn is_safe_relative_path_rejects_escapes() {
        use crate::workspace::is_safe_relative_path as safe;
        assert!(safe(""));
        assert!(safe("skills"));
        assert!(safe("a/b/c"));
        assert!(safe("./a"));
        assert!(!safe("/abs"));
        assert!(!safe("\\abs"));
        assert!(!safe("C:\\\\x"));
        assert!(!safe("C:/x"));
        assert!(!safe("\\\\server\\share"));
        assert!(!safe("a/../b"));
        assert!(!safe("../up"));
        assert!(safe("a/./b"));
    }
}
