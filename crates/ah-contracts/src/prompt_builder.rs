//! prompt-builder seam:系统提示构建器(对齐 core/single_agent/prompts/builder.py)。
//!
//! 纯类型契约:多语言 PromptSection + 注册/渲染 + 诊断报告 + 注入清洗。

/// 语言代码("cn"/"en")。
pub const DEFAULT_LANGUAGE: &str = "cn";
pub const SUPPORTED_LANGUAGES: [&str; 2] = ["cn", "en"];

/// 单个提示 section(多语言内容 + 优先级;对齐 PromptSection)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromptSection {
    pub name: String,
    pub content: std::collections::BTreeMap<String, String>,
    pub priority: u64,
}

impl PromptSection {
    pub fn new(
        name: impl Into<String>,
        content: std::collections::BTreeMap<String, String>,
        priority: u64,
    ) -> Self {
        Self {
            name: name.into(),
            content,
            priority,
        }
    }

    /// 按语言渲染;缺失回退 DEFAULT_LANGUAGE,再回退首个值,全空 → 空串。
    pub fn render(&self, language: &str) -> String {
        if let Some(c) = self.content.get(language) {
            return c.clone();
        }
        if let Some(c) = self.content.get(DEFAULT_LANGUAGE) {
            return c.clone();
        }
        self.content.values().next().cloned().unwrap_or_default()
    }

    pub fn char_count(&self, language: &str) -> usize {
        self.render(language).chars().count()
    }
}

/// Section 名常量(对齐 SectionName)。
pub mod section_name {
    pub const IDENTITY: &str = "identity";
    pub const SAFETY: &str = "safety";
    pub const SKILLS: &str = "skills";
    pub const TOOLS: &str = "tools";
    pub const TODO: &str = "todo";
    pub const TASK_TOOL: &str = "task_tool";
    pub const TOOL_NAVIGATION: &str = "tool_navigation";
    pub const PROGRESSIVE_TOOL_RULES: &str = "progressive_tool_rules";
    pub const RUNTIME: &str = "runtime";
    pub const PROMPT_ATTACHMENTS: &str = "prompt_attachments";
    pub const MEMORY: &str = "memory";
    pub const SESSION_TOOLS: &str = "session_tools";
    pub const MODE_INSTRUCTIONS: &str = "mode_instructions";
    pub const WORKSPACE: &str = "workspace";
    pub const HEARTBEAT: &str = "heartbeat";
    pub const CONTEXT: &str = "context";
    pub const EXTERNAL_MEMORY: &str = "external_memory";
    pub const COMPLETION_SIGNAL: &str = "completion_signal";
    pub const VERIFICATION_CONTRACT: &str = "verification_contract";
    pub const EVOLUTION_PROTOCOL: &str = "evolution_protocol";
    pub const EVOLUTION_TEAM_PROTOCOL: &str = "evolution_team_protocol";
    pub const SKILL_CREATION_GUIDANCE: &str = "skill_creation_guidance";
    pub const SKILL_CREATION_NUDGE: &str = "skill_creation_nudge";
    pub const TEAM_SKILL_CREATION_GUIDANCE: &str = "team_skill_creation_guidance";
    pub const TEAM_SKILL_CREATION_NUDGE: &str = "team_skill_creation_nudge";
    pub const GOAL_PROTOCOL: &str = "goal_protocol";
}

/// 提示构建模式(对齐 PromptMode)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    #[default]
    Full,
    Minimal,
    None,
}

/// 诊断报告中的单 section 快照(对齐 SectionInfo)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SectionInfo {
    pub name: String,
    pub priority: u64,
    pub char_count: usize,
}

/// 提示诊断报告(对齐 PromptReport)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromptReport {
    pub total_chars: usize,
    pub estimated_tokens: usize,
    pub section_count: usize,
    pub sections: Vec<SectionInfo>,
    pub mode: String,
    pub language: String,
}

/// 注入清洗错误(预留;当前清洗是纯函数无错误)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSanitizeError(pub String);

impl core::fmt::Display for PromptSanitizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PromptSanitizeError {}
