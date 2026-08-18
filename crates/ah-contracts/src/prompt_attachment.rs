//! prompt-attachment seam:prompt 附件核心(对齐 openjiuwen/harness/prompts/
//! prompt_attachment_manager.py 的纯函数部分:数据模型 + 哈希/净化接口)。
//!
//! 契约层零实现,只声明:
//! - [`PromptAttachmentKind`]:内置附件类型枚举(serde snake_case);
//! - [`PromptAttachment`]:结构化动态 prompt 片段(默认 kind=Generic、priority=100、
//!   content_kind="text/plain");
//! - `PromptAttachmentUpdate`:允许修改的字段显式更新模型;
//! - [`PromptAttachmentApi`]:sha256 内容哈希、渲染哈希、语义哈希与安全 id 净化的
//!   纯函数 Service Definition;
//! - `AttachmentError`:附件域错误(纯数据)。
//!
//! 实现(sha256、canonical JSON 等)全部位于插件 crate,不进入本文件。

use crate::seam::Seam;
use serde::{Deserialize, Serialize};

/// 内置 prompt 附件类型(对齐 Python `PromptAttachmentKind`,serde snake_case:
/// `todo_reminder` / `workspace_delta`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptAttachmentKind {
    /// 通用附件。
    #[default]
    Generic,
    /// 纯文本片段。
    Text,
    /// 运行时信息。
    Runtime,
    /// 记忆内容。
    Memory,
    /// 文件引用。
    File,
    /// 工具相关。
    Tool,
    /// 技能相关。
    Skill,
    /// 诊断信息。
    Diagnostic,
    /// todo 提醒。
    TodoReminder,
    /// 工作区差异。
    WorkspaceDelta,
}

/// 默认优先级(对齐 Python `priority: int = 100`)。
fn default_priority() -> i32 {
    100
}

/// 默认内容媒体类型(对齐 Python `content_kind: str = "text/plain"`)。
fn default_content_kind() -> String {
    "text/plain".to_string()
}

/// 结构化动态 prompt 片段,注入单次模型调用(对齐 Python `PromptAttachment`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptAttachment {
    /// 稳定附件 id。
    pub id: String,
    /// 归属 section(净化后的稳定片段名)。
    pub section: String,
    /// 附件类型;默认 [`PromptAttachmentKind::Generic`]。
    #[serde(default)]
    pub kind: PromptAttachmentKind,
    /// 附件正文;None 表示无内容。
    #[serde(default)]
    pub content: Option<String>,
    /// 排序优先级(越小越靠前);默认 100。
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// 来源标识。
    #[serde(default)]
    pub source: Option<String>,
    /// 归属会话 id。
    pub session_id: String,
    /// 创建时间(规范 ISO-8601 字符串)。
    #[serde(default)]
    pub created_at: Option<String>,
    /// 最近更新时间。
    #[serde(default)]
    pub updated_at: Option<String>,
    /// 过期时间(到达即视为过期)。
    #[serde(default)]
    pub expires_at: Option<String>,
    /// 附加元数据(任意 JSON 对象)。
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    /// 内容媒体类型;默认 "text/plain"。
    #[serde(default = "default_content_kind")]
    pub content_kind: String,
    /// 内容外部路径(文件引用时使用)。
    #[serde(default)]
    pub content_path: Option<String>,
    /// 内容 sha256(写入时计算)。
    #[serde(default)]
    pub content_sha256: Option<String>,
}

/// 显式更新模型:只包含调用方允许修改的字段(对齐 Python `PromptAttachmentUpdate`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptAttachmentUpdate {
    /// 新附件类型。
    pub kind: Option<PromptAttachmentKind>,
    /// 新正文。
    pub content: Option<String>,
    /// 新优先级。
    pub priority: Option<i32>,
    /// 新来源。
    pub source: Option<String>,
    /// 新过期时间。
    pub expires_at: Option<String>,
    /// 新元数据(整体替换)。
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
    /// 新内容媒体类型。
    pub content_kind: Option<String>,
}

/// prompt 附件 Service Definition:纯函数哈希与 id 净化。
pub trait PromptAttachmentApi: Seam {
    /// 内容 sha256;`None` 视为空串(对齐 `_content_sha256`)。
    fn content_sha256(&self, content: Option<&str>) -> String;

    /// 渲染文本的稳定 sha256(对齐 `hash_rendered`)。
    fn hash_rendered(&self, rendered: &str) -> String;

    /// 语义哈希:排除 `content_sha256` / `created_at` / `updated_at` 三个字段后,
    /// 对 canonical JSON(键排序、紧凑 `{",", ":"}`、`ensure_ascii=false`)取 sha256
    /// (对齐 `hash_prompt_attachment`)。
    fn hash_attachment(&self, attachment: &PromptAttachment) -> String;

    /// 安全 id 片段净化(对齐 `_safe_id_part`):
    /// 非 `[A-Za-z0-9_.-]` 连续字符 → `_`;strip 两端 `._-`;空 → `fallback`;
    /// 超 80 截断;全部非法 → `sha256(raw)` 前 12 位。
    fn safe_id_part(&self, value: Option<&str>, fallback: &str) -> String;
}

/// prompt 附件域错误(纯数据,供实现/消费方显式报错)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentError(pub String);

impl core::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AttachmentError {}
