//! Workspace 动态 section(对齐 harness/prompts/sections/workspace.py)。
//!
//! 纯逻辑:目录描述/重要文件表/头部常量 + 目录树格式化 + 内容组装;
//! 真实目录扫描(sys_operation.fs)留待集成,树构建接受预扫描节点。

use ah_contracts::prompt_builder::{PromptSection, section_name};
use std::collections::BTreeMap;

pub const WORKSPACE_HEADER_CN: &str = "# 工作空间\n\n";
pub const WORKSPACE_HEADER_EN: &str = "# Workspace\n\n";

pub const IMPORTANT_FILES_CN: &str = "## 工作目录下重要文件\n\n| 文件 | 用途 | 操作工具 |\n|------|------|----------|\n| `AGENT.md` | Agent 启动指南 | read_file |\n| `IDENTITY.md` | Agent 身份设定（名字、角色定位、用户为 Agent 指定的称呼） | read_file / edit_file |\n| `USER.md` | 用户本人档案（姓名、职业、爱好等） | read_memory / write_memory / edit_memory |\n| `memory/MEMORY.md` | 长期记忆（决策、偏好、持久事实） | read_memory / write_memory / edit_memory |\n| `memory/daily_memory/YYYY-MM-DD.md` | 每日会话记录 | read_memory / write_memory / edit_memory |\n";
pub const IMPORTANT_FILES_EN: &str = "## Important Files in Working Directory\n\n| File | Purpose | Tools |\n|------|---------|-------|\n| `AGENT.md` | Agent startup guide | read_file |\n| `IDENTITY.md` | Agent identity settings (name, role, user-assigned agent name) | read_file / edit_file |\n| `USER.md` | User's own profile (name, occupation, hobbies, etc.) | read_memory / write_memory / edit_memory |\n| `memory/MEMORY.md` | Long-term memory (decisions, preferences, persistent facts) | read_memory / write_memory / edit_memory |\n| `memory/daily_memory/YYYY-MM-DD.md` | Daily session records | read_memory / write_memory / edit_memory |\n";

/// 目录/文件描述(对齐 DIRECTORY_DESCRIPTIONS)。
pub fn directory_descriptions_cn() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("AGENT.md", "智能体配置"),
        ("SOUL.md", "灵魂与价值观"),
        ("HEARTBEAT.md", "心跳任务"),
        ("USER.md", "用户信息"),
        ("IDENTITY.md", "身份凭证"),
        ("MEMORY.md", "长期记忆"),
        ("memory", "记忆核心模块"),
        ("daily_memory", "每日结构化记忆"),
        ("todo", "待办事项"),
        ("messages", "消息历史"),
        ("skills", "技能库"),
        ("agents", "子智能体"),
    ])
}

pub fn directory_descriptions_en() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("AGENT.md", "Agent configuration"),
        ("SOUL.md", "Soul & values"),
        ("HEARTBEAT.md", "Heartbeat tasks"),
        ("USER.md", "User information"),
        ("IDENTITY.md", "Identity credentials"),
        ("MEMORY.md", "Long-term memory"),
        ("memory", "Memory core module"),
        ("daily_memory", "Daily structured memory"),
        ("todo", "Todo items"),
        ("messages", "Message history"),
        ("skills", "Skills library"),
        ("agents", "Sub-agents"),
    ])
}

/// 目录/文件描述查找(对齐 _get_directory_description;未知返回空串)。
pub fn get_directory_description(name: &str, language: &str) -> String {
    let descs = if language == "en" {
        directory_descriptions_en()
    } else {
        directory_descriptions_cn()
    };
    descs.get(name).map(|s| s.to_string()).unwrap_or_default()
}

/// 目录树节点(对齐 DirNode)。
#[derive(Debug, Clone, PartialEq)]
pub struct DirNode {
    pub name: String,
    pub path: String,
    pub description: String,
    pub is_file: bool,
    pub children: Vec<DirNode>,
}

impl DirNode {
    pub fn dir(name: &str, path: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            is_file: false,
            children: Vec::new(),
        }
    }

    pub fn file(name: &str, path: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            description: description.to_string(),
            is_file: true,
            children: Vec::new(),
        }
    }
}

/// 单个节点格式化并递归子节点(对齐 _format_node)。
fn format_node(node: &DirNode, lines: &mut Vec<String>, prefix: &str, is_last: bool) {
    let connector = if is_last { "└── " } else { "├── " };
    let line = if node.is_file {
        format!("{prefix}{connector}{}", node.name)
    } else {
        let base = format!("{prefix}{connector}{}/", node.name);
        if node.description.is_empty() {
            base
        } else {
            format!("{base}  # {}", node.description)
        }
    };
    lines.push(line);

    let child_count = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == child_count - 1;
        let child_prefix = if child_is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };
        format_node(child, lines, &child_prefix, child_is_last);
    }
}

/// 顶层节点树格式化(对齐 _format_tree)。
pub fn format_tree(nodes: &[DirNode]) -> Vec<String> {
    let mut lines = Vec::new();
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        format_node(node, &mut lines, "", i == count - 1);
    }
    lines
}

/// 工作空间内容(头部 + 路径声明 + 重要文件表;对齐 build_workspace_content)。
pub fn build_workspace_content(root_path: &str, language: &str) -> String {
    let header = if language == "en" {
        WORKSPACE_HEADER_EN
    } else {
        WORKSPACE_HEADER_CN
    };
    let important = if language == "en" {
        IMPORTANT_FILES_EN
    } else {
        IMPORTANT_FILES_CN
    };
    let path_line = if language == "en" {
        format!("Your working directory is: `{root_path}`\n\n{important}")
    } else {
        format!("你的工作目录是：`{root_path}`\n\n{important}")
    };
    format!("{header}{path_line}")
}

/// 构建 workspace section(priority=70;对齐 build_workspace_section)。
pub fn build_workspace_section(root_path: &str, language: &str) -> PromptSection {
    let content = build_workspace_content(root_path, language);
    let mut map = BTreeMap::new();
    map.insert(language.to_string(), content);
    PromptSection::new(section_name::WORKSPACE, map, 70)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_descriptions_bilingual() {
        assert_eq!(get_directory_description("memory", "cn"), "记忆核心模块");
        assert_eq!(
            get_directory_description("memory", "en"),
            "Memory core module"
        );
        assert_eq!(get_directory_description("unknown", "cn"), "");
    }

    #[test]
    fn tree_formatting_lines() {
        let mut skills = DirNode::dir(
            "skills",
            "skills",
            &get_directory_description("skills", "cn"),
        );
        skills
            .children
            .push(DirNode::file("SKILL.md", "skills/SKILL.md", ""));
        let nodes = vec![
            DirNode::dir(
                "memory",
                "memory",
                &get_directory_description("memory", "cn"),
            ),
            skills,
        ];
        let lines = format_tree(&nodes);
        assert_eq!(lines[0], "├── memory/  # 记忆核心模块");
        assert_eq!(lines[1], "└── skills/  # 技能库");
        assert_eq!(lines[2], "    └── SKILL.md");
    }

    #[test]
    fn tree_formats_single_node() {
        let nodes = vec![DirNode::file("AGENT.md", "AGENT.md", "")];
        let lines = format_tree(&nodes);
        assert_eq!(lines, vec!["└── AGENT.md"]);
    }

    #[test]
    fn workspace_content_assembly() {
        let cn = build_workspace_content("/tmp/proj", "cn");
        assert!(cn.starts_with("# 工作空间"));
        assert!(cn.contains("你的工作目录是：`/tmp/proj`"));
        assert!(cn.contains("| `AGENT.md` | Agent 启动指南 | read_file |"));
        let en = build_workspace_content("/tmp/proj", "en");
        assert!(en.starts_with("# Workspace"));
        assert!(en.contains("Your working directory is: `/tmp/proj`"));
        assert!(en.contains("| `AGENT.md` | Agent startup guide | read_file |"));
    }

    #[test]
    fn workspace_section_build() {
        let section = build_workspace_section("/tmp/proj", "cn");
        assert_eq!(section.name, section_name::WORKSPACE);
        assert_eq!(section.priority, 70);
        assert!(section.render("cn").contains("工作空间"));
    }
}
