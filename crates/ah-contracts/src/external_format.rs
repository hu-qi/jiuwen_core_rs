//! external-format seam:外部 CLI 入站渲染(对齐 openjiuwen/agent_teams/external/format.py)。
//!
//! 组合层:把入站消息/任务板渲染成与进程内 dispatcher 一致的 XML:
//! - render_message:一条入站消息 → <team-inbound>(+ reply-hint 或 hitt-silence note,
//!   框架模板消息 body 传入则去掉 reply hint);
//! - render_task_line:一行任务看板条目(带相对时间);
//! - render_task_board:<team-event kind=\"task-board\">(角色化标题 + 非终态任务逐行);
//! - 终态任务(status ∈ {completed, cancelled})在看板中过滤。
//!
//! 依赖 inbound-render 的 InboundRender 与 timefmt 的 Timefmt seam(经参数注入,调用方
//! 从 ctx 解析);i18n 文案(reply hint / 标题)由调用方传入已解析文本。
//! 契约零实现:组合逻辑由插件提供(如 ah-plugins-external-format)。

use crate::inbound_render::InboundRender;
use crate::seam::Seam;
use crate::timefmt::Timefmt;

/// 终态任务状态(看板过滤;对齐 _TERMINAL_TASK_STATUSES)。
pub const TERMINAL_TASK_STATUSES: &[&str] = &["completed", "cancelled"];

/// 消息行结构视图(对齐 _MessageLike)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageView {
    pub broadcast: bool,
    pub timestamp: i64,
    pub from_member_name: String,
    pub message_id: String,
    pub content: String,
}

/// 任务看板行结构视图(对齐 _TaskLike)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskLineView {
    pub task_id: String,
    pub title: String,
    pub content: String,
    pub status: String,
    pub assignee: Option<String>,
    pub updated_at: Option<i64>,
}

/// external-format Seam(Service Definition):组合渲染。
pub trait ExternalFormat: Seam {
    /// 渲染一条入站消息为 <team-inbound>(+ note)。
    /// reply_hint / hitt_silence_note 为调用方经 i18n 解析的文本。
    #[allow(clippy::too_many_arguments)]
    fn render_message(
        &self,
        message: &MessageView,
        is_human_agent: bool,
        now_ms: i64,
        body: Option<&str>,
        reply_hint: Option<&str>,
        hitt_silence_note: Option<&str>,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String;

    /// 渲染一批入站消息(bodies 按 message_id 提供框架模板正文)。
    #[allow(clippy::too_many_arguments)]
    fn render_messages(
        &self,
        messages: &[MessageView],
        is_human_agent: bool,
        now_ms: i64,
        bodies: Option<&std::collections::HashMap<String, String>>,
        reply_hint: Option<&str>,
        hitt_silence_note: Option<&str>,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String;

    /// 渲染一行任务看板条目(带相对时间)。
    fn render_task_line(
        &self,
        task: &TaskLineView,
        now_ms: i64,
        unassigned_marker: &str,
        timefmt: &dyn Timefmt,
    ) -> String;

    /// 渲染任务看板为 <team-event kind=\"task-board\">;无行动项返回空串。
    #[allow(clippy::too_many_arguments)]
    fn render_task_board(
        &self,
        tasks: &[TaskLineView],
        is_leader: bool,
        now_ms: i64,
        leader_header: &str,
        teammate_header: &str,
        unassigned_marker: &str,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String;
}
