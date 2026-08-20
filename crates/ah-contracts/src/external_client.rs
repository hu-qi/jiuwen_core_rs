//! external-client seam:进程边界外部成员客户端(对齐
//! `openjiuwen/agent_teams/external/client.py`)。
//!
//! `ExternalTeamClient` 让外部 agent(第三方 CLI / 独立服务)以一等团队成员身份
//! 接入团队:经共享 DB + messager 执行与进程内成员相同的协作操作。本文件声明:
//! - `InboxView`:收件箱快照(未读 direct+broadcast 消息 + 可行动任务板)+ `is_empty`;
//! - `InboxMessage`:消息行 + 可选 meta(框架模板展开需要,对齐 TeamMessageBase.meta);
//! - `BROADCAST_TARGET = "*"`(广播路由哨兵);
//! - `compose_inbox_text`:read_inbox 的确定性文本组装(渲染段 → "\n\n" 连接,
//!   空 → "(inbox empty)");
//! - `ExternalInboxSource` seam:收件箱数据源(未读消息查询 / 标记已读 / 任务列表 /
//!   任务·成员行读取,对齐 TeamMessageManager/TeamTaskManager 的读表面);
//! - `ExternalTeamClientFactory` seam:按 TeamJoinDescriptor 构建客户端
//!   (对齐 `ExternalTeamClient(descriptor)` 构造 + MCP server 每连接构建);
//! - `ExternalTeamClient` seam:客户端状态机(session 上下文绑定 / 幂等 connect·close /
//!   未连接显式报错)+ fetch_inbox / read_inbox / watch。
//!
//! 契约零实现:数据源由团队运行时提供,组装/状态逻辑由插件实现。

use crate::external_format::{MessageView, TaskLineView};
use crate::seam::Seam;
use crate::team_join_descriptor::TeamJoinDescriptor;
use async_trait::async_trait;
use std::sync::Arc;

/// 广播路由哨兵(对齐 `BROADCAST_TARGET = "*"`):send_message 的 `to="*"`。
pub const BROADCAST_TARGET: &str = "*";

/// 一条收件箱消息(渲染视图 + 框架模板 meta)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InboxMessage {
    /// 渲染用消息视图(对齐 _MessageLike)。
    pub view: MessageView,
    /// meta 列(框架模板消息;None/畸形 = 普通消息)。
    #[serde(default)]
    pub meta: Option<String>,
}

/// 收件箱快照(对齐 `InboxView`)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InboxView {
    /// 发给该成员且未读的 direct + broadcast 消息。
    pub messages: Vec<InboxMessage>,
    /// 当前团队全部任务(可行动任务板;终态由看板渲染过滤)。
    pub tasks: Vec<TaskLineView>,
}

impl InboxView {
    /// 是否没有需要成员行动的内容(对齐 `is_empty`)。
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.tasks.is_empty()
    }
}

/// 组装 read_inbox 文本(对齐 `read_inbox` 的尾部组合逻辑)。
///
/// `messages_text` 为已渲染的消息块(空 = 无消息),`board_text` 为已渲染的
/// 任务板块(空 = 无行动项);两者经 "\n\n" 连接,全空 → "(inbox empty)"。
pub fn compose_inbox_text(messages_text: &str, board_text: &str) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(2);
    if !messages_text.is_empty() {
        parts.push(messages_text);
    }
    if !board_text.is_empty() {
        parts.push(board_text);
    }
    if parts.is_empty() {
        "(inbox empty)".to_string()
    } else {
        parts.join("\n\n")
    }
}

/// 外部客户端错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalClientError(pub String);

impl core::fmt::Display for ExternalClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExternalClientError {}

/// 收件箱数据源 Seam(Service Definition):外部客户端读表面。
///
/// 对齐 Python `TeamMessageManager` / `TeamTaskManager` 的外部读操作:
/// - 未读 direct / broadcast 消息查询与标记已读;
/// - 团队任务列表;
/// - 任务 / 成员行读取(供 `_expand_template_bodies` 渲染框架模板消息,F_63)。
#[async_trait]
pub trait ExternalInboxSource: Seam {
    /// 发给该成员且未读的 direct 消息。
    async fn unread_direct_messages(&self, member_name: &str) -> Vec<InboxMessage>;

    /// 发给该成员且未读的 broadcast 消息。
    async fn unread_broadcast_messages(&self, member_name: &str) -> Vec<InboxMessage>;

    /// 标记一条消息已读(对齐 `mark_message_read(message_id, member_name)`)。
    async fn mark_message_read(&self, message_id: &str, member_name: &str);

    /// 当前团队全部任务(供看板渲染,对齐 `list_tasks()` 无状态过滤)。
    async fn list_tasks(&self) -> Vec<TaskLineView>;

    /// 按 id 读任务行(None = 不存在;供模板展开)。
    async fn get_task(&self, task_id: &str) -> Option<crate::team_message::TaskView>;

    /// 按名读成员行(None = 不存在;供模板展开)。
    async fn get_member(&self, member_name: &str) -> Option<crate::team_message::MemberView>;
}

/// 外部团队成员客户端 Seam(Service Definition)。
///
/// 对齐 `ExternalTeamClient` 的可观察表面:描述符投影、session 上下文绑定、
/// 幂等 connect/close、fetch_inbox / read_inbox / watch。消息/任务写操作
/// (send_message / create_task / claim / complete / update / list_members)
/// 走 `teams` seam 的 `TeamRuntime`(见 ah-contracts/src/teams.rs),不在此重复。
#[async_trait]
pub trait ExternalTeamClient: Seam {
    /// 绑定的团队 session id(对齐 `session_id` 属性)。
    fn session_id(&self) -> &str;

    /// 团队运行时语言(对齐 `language` 属性)。
    fn language(&self) -> &str;

    /// 该客户端服务的成员身份(对齐 `member_name` 属性)。
    fn member_name(&self) -> &str;

    /// 目标团队标识(对齐 `team_name` 属性)。
    fn team_name(&self) -> &str;

    /// 是否 leader 角色(对齐 `is_leader`)。
    fn is_leader(&self) -> bool;

    /// 是否 human-agent avatar(对齐 `is_human_agent`,驱动入站 note)。
    fn is_human_agent(&self) -> bool;

    /// 外部接入场景:"member" / "operator"(对齐 `scope`)。
    fn scope(&self) -> &str;

    /// 是否已连接(connect 成功且未 close)。
    fn is_connected(&self) -> bool;

    /// 挂接收件箱数据源并进入已连接状态(幂等:重复调用为 no-op)。
    ///
    /// 对齐 `connect()` 的数据面:为 member scope 构建真实 teammate 工具集
    /// (工具构建由调用方经 `tools` seam 完成,此处只接线数据源)。
    fn connect(&self, source: Arc<dyn ExternalInboxSource>) -> Result<(), ExternalClientError>;

    /// 释放会话上下文并回到未连接状态(幂等)。
    fn close(&self) -> Result<(), ExternalClientError>;

    /// 重断言 session-id + 语言上下文(对齐 `bind_session_context`)。
    ///
    /// 每个操作在自己的任务上下文运行时(如 MCP server 每工具调用一个 task)
    /// 必须先调用;`set_session_id` / `set_language` 由实现方经注入的
    /// TeamSessionContext / TeamI18n seam 执行。
    fn bind_session_context(&self) -> Result<(), ExternalClientError>;

    /// 读未读消息 + 当前任务板(对齐 `fetch_inbox`)。
    ///
    /// `mark_read=true`(默认)时未读消息在返回前被标记已读,后续轮询不会重复投递。
    async fn fetch_inbox(&self, mark_read: bool) -> Result<InboxView, ExternalClientError>;

    /// 渲染未读消息 + 任务板为一个文本块(对齐 `read_inbox`)。
    ///
    /// 组合 external-format(render_messages / render_task_board)+ team-message
    /// (框架模板消息展开)+ i18n 文案;空收件箱 → "(inbox empty)"。
    async fn read_inbox(&self, mark_read: bool) -> Result<String, ExternalClientError>;

    /// 阻塞监听团队事件,每有相关事件以新收件箱回调 observer(对齐 `watch`)。
    ///
    /// 订阅 MESSAGE + TASK topic;每个相关事件都触发一次收件箱重取,空收件箱
    /// 不回调。运行到所在任务被取消为止。
    async fn watch(&self, observer: &dyn InboxObserver) -> Result<(), ExternalClientError>;
}

/// 收件箱观察者(对齐 `InboxObserver = Callable[[InboxView], Awaitable[None]]`)。
#[async_trait]
pub trait InboxObserver: Send + Sync {
    /// 收到新收件箱快照时回调。
    async fn on_inbox(&self, view: InboxView);
}

/// 外部客户端工厂 Seam(Service Definition):按描述符构建客户端。
///
/// 对齐 `ExternalTeamClient(descriptor)` 构造:每个连接(MCP server / 外部 CLI)
/// 从 `TeamJoinDescriptor` 构建一个独立客户端实例。
pub trait ExternalTeamClientFactory: Seam {
    /// 构建绑定到描述符的客户端。
    fn build(&self, descriptor: &TeamJoinDescriptor) -> Arc<dyn ExternalTeamClient>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(messages: usize, tasks: usize) -> InboxView {
        InboxView {
            messages: (0..messages)
                .map(|i| InboxMessage {
                    view: MessageView {
                        broadcast: false,
                        timestamp: 1_700_000_000_000 + i as i64,
                        from_member_name: format!("sender-{i}"),
                        message_id: format!("m-{i}"),
                        content: format!("body-{i}"),
                    },
                    meta: None,
                })
                .collect(),
            tasks: (0..tasks)
                .map(|i| TaskLineView {
                    task_id: format!("t-{i}"),
                    title: format!("Title {i}"),
                    content: "body".to_string(),
                    status: "pending".to_string(),
                    assignee: None,
                    updated_at: Some(1_700_000_000_000),
                })
                .collect(),
        }
    }

    #[test]
    fn inbox_is_empty_matches_both_fields() {
        assert!(view(0, 0).is_empty());
        assert!(!view(1, 0).is_empty());
        assert!(!view(0, 1).is_empty());
        assert!(!view(1, 1).is_empty());
    }

    #[test]
    fn compose_joins_non_empty_parts_and_falls_back() {
        assert_eq!(compose_inbox_text("", ""), "(inbox empty)");
        assert_eq!(compose_inbox_text("MSG", ""), "MSG");
        assert_eq!(compose_inbox_text("", "BOARD"), "BOARD");
        assert_eq!(compose_inbox_text("MSG", "BOARD"), "MSG\n\nBOARD");
    }

    #[test]
    fn broadcast_target_is_star() {
        assert_eq!(BROADCAST_TARGET, "*");
    }
}
