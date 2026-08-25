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

/// 附件存储查询过滤(对齐 `list_by_filter` / `remove_by_filter` 参数)。
#[derive(Debug, Clone, Default)]
pub struct AttachmentFilter {
    pub session_id: Option<String>,
    pub section: Option<String>,
    pub kind: Option<PromptAttachmentKind>,
    pub source: Option<String>,
}

/// prompt 附件管理 Seam(对齐 `PromptAttachmentManager` 的确定性 CRUD 部分)。
///
/// 存储为内存态(session_id → section → 附件),所有写路径统一走
/// `_normalize_for_write`(时间戳/内容哈希/metadata.section 注入),id 固定为
/// `session.{safe(session_id)}.{safe(section)}`。
pub trait PromptAttachmentStore: Seam {
    /// 添加或替换一个 section(对齐 `add_section`):metadata 合并
    /// `{section, source}`;id 由 session+section 生成。
    #[allow(clippy::too_many_arguments)] // 镜像 Python add_section 关键字签名。
    fn add_section(
        &self,
        session_id: &str,
        section: &str,
        content: &str,
        kind: PromptAttachmentKind,
        source: &str,
        priority: i32,
        metadata: Option<&serde_json::Map<String, serde_json::Value>>,
        content_kind: &str,
        expires_at: Option<&str>,
    ) -> Result<PromptAttachment, AttachmentError>;

    /// 清空一个 section;返回删除条数(0/1)(对齐 `clear_section`)。
    fn clear_section(&self, session_id: &str, section: &str) -> usize;

    /// 按 id 取回附件(深拷贝);session_id 约束可选(对齐 `get_by_id`)。
    fn get_by_id(
        &self,
        prompt_attachment_id: &str,
        session_id: Option<&str>,
    ) -> Option<PromptAttachment>;

    /// 按 id 更新(对齐 `update_by_id`);未找到显式 KeyError 语义错误。
    fn update_by_id(
        &self,
        prompt_attachment_id: &str,
        update: &PromptAttachmentUpdate,
    ) -> Result<PromptAttachment, AttachmentError>;

    /// 按 id 删除;未找到或 session 不匹配返回 false(对齐 `remove_by_id`)。
    fn remove_by_id(&self, prompt_attachment_id: &str, session_id: Option<&str>) -> bool;

    /// 按过滤列出(稳定排序)(对齐 `list_by_filter`)。
    fn list_by_filter(&self, filter: &AttachmentFilter) -> Vec<PromptAttachment>;

    /// 按过滤删除;无任何过滤且 allow_all=false 显式错误(对齐 `remove_by_filter`)。
    fn remove_by_filter(
        &self,
        filter: &AttachmentFilter,
        allow_all: bool,
    ) -> Result<usize, AttachmentError>;

    /// 清空一个会话的全部附件;返回删除条数(对齐 `clear_session`)。
    fn clear_session(&self, session_id: &str) -> usize;

    /// 清空全部附件;返回删除条数(对齐 `clear_all`)。
    fn clear_all(&self) -> usize;

    /// 收集会话可见附件(剔除过期;返回稳定排序)(对齐 `collect_for_session`)。
    fn collect_for_session(&self, session_id: &str) -> Vec<PromptAttachment>;
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

// ---------------------------------------------------------------------------
// 渲染纯函数(对齐 prompt_attachment_manager.py 的 `_xml_text` / `_xml_attr` /
// `render` / `_stable_sort` / `_is_expired` / `_make_section_id` 确定性部分)
// ---------------------------------------------------------------------------

/// XML 文本转义(对齐 `html.escape(text, quote=False)`):转义 `&` `<` `>`,
/// 不转义引号。
pub fn xml_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// XML 属性转义(对齐 `html.escape(str(text), quote=True).replace("'", "&apos;")`):
/// 额外转义 `"` 为 `&quot;`、`'` 为 `&apos;`;None 视为空串。
pub fn xml_attr(text: Option<&str>) -> String {
    let raw = text.unwrap_or("");
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// 附件类型字符串值(对齐 `_kind_value`)。
pub fn kind_value(kind: &PromptAttachmentKind) -> &'static str {
    match kind {
        PromptAttachmentKind::Generic => "generic",
        PromptAttachmentKind::Text => "text",
        PromptAttachmentKind::Runtime => "runtime",
        PromptAttachmentKind::Memory => "memory",
        PromptAttachmentKind::File => "file",
        PromptAttachmentKind::Tool => "tool",
        PromptAttachmentKind::Skill => "skill",
        PromptAttachmentKind::Diagnostic => "diagnostic",
        PromptAttachmentKind::TodoReminder => "todo_reminder",
        PromptAttachmentKind::WorkspaceDelta => "workspace_delta",
    }
}

/// 稳定排序键(对齐 `_stable_sort` 的 key:`(priority, source or "", section)`)。
pub fn stable_sort_key(attachment: &PromptAttachment) -> (i32, String, String) {
    (
        attachment.priority,
        attachment.source.clone().unwrap_or_default(),
        attachment.section.clone(),
    )
}

/// 附件是否已过期(对齐 `_is_expired`:expires_at 非空且 <= now)。
pub fn is_expired(attachment: &PromptAttachment, now: &str) -> bool {
    matches!(&attachment.expires_at, Some(expires) if expires.as_str() <= now)
}

/// 默认最大单附件字符数(对齐 `_DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS`)。
pub const DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS: usize = 12000;
/// 默认最大渲染总字符数(对齐 `_DEFAULT_MAX_RENDERED_CHARS`)。
pub const DEFAULT_MAX_RENDERED_CHARS: usize = 48000;

/// 渲染 prompt 附件为 user-role system-reminder 块(对齐 `PromptAttachmentManager.render`)。
///
/// 已按稳定排序键排序输入;空列表返回空串。单附件超限截断并在尾部追加
/// `[Prompt attachment truncated: ...]`;渲染总量超限截断并重写截断标记。
pub fn render(
    attachments: &[PromptAttachment],
    max_prompt_attachment_chars: usize,
    max_rendered_chars: usize,
) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&PromptAttachment> = attachments.iter().collect();
    sorted.sort_by_key(|item| stable_sort_key(item));

    let mut truncated_ids: Vec<String> = Vec::new();
    let mut blocks: Vec<String> = vec![
        "The following context is automatically attached for this model call only.".to_string(),
        "It may or may not be relevant to your tasks. Do not respond to it unless it highly relevant to your task.".to_string(),
        String::new(),
    ];
    for item in &sorted {
        let mut content = item.content.clone().unwrap_or_default();
        if max_prompt_attachment_chars > 0 && content.chars().count() > max_prompt_attachment_chars
        {
            content = content
                .chars()
                .take(max_prompt_attachment_chars)
                .collect::<String>()
                + "\n\n[Prompt attachment truncated: content exceeded max_prompt_attachment_chars.]";
            truncated_ids.push(item.id.clone());
        }
        blocks.push(format!(
            "<prompt-attachment type=\"{}\">",
            xml_attr(Some(kind_value(&item.kind)))
        ));
        blocks.push(xml_text(&content));
        blocks.push("</prompt-attachment>".to_string());
        blocks.push(String::new());
    }

    let body = blocks.join("\n").trim_end().to_string();
    let mut rendered = format!("<system-reminder>\n{body}\n</system-reminder>");
    if max_rendered_chars > 0 && rendered.chars().count() > max_rendered_chars {
        rendered = rendered
            .chars()
            .take(max_rendered_chars)
            .collect::<String>()
            + "\n\n[Prompt attachments truncated: rendered content exceeded max_rendered_chars.]\n"
            + "</system-reminder>";
        truncated_ids = sorted.iter().map(|item| item.id.clone()).collect();
    }

    let _ = truncated_ids; // 日志由插件层发出。
    rendered
}

/// 注入渲染文本为独立 user 消息(对齐 `inject_messages`):空渲染返回原消息副本。
pub fn inject_messages<S: AsRef<str>>(
    messages: &[S],
    rendered_prompt_attachments: &str,
) -> Vec<String> {
    let mut new_messages: Vec<String> = messages.iter().map(|m| m.as_ref().to_string()).collect();
    if !rendered_prompt_attachments.is_empty() {
        new_messages.push(rendered_prompt_attachments.to_string());
    }
    new_messages
}
