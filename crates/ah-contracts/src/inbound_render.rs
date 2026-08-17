//! inbound-render seam:入站团队消息/框架事件/团队状态 → XML 渲染(对齐 openjiuwen/agent_teams/inbound_render.py,F_46)。
//!
//! 成员的 harness 输入混合两类内容:其他成员/用户发的原始消息,与运行时附加的框架
//! 元数据(消息 id / 时间 / reply hint / 事件通知)。本 seam 把它们渲染为语义化 XML:
//! - `<team-inbound>` 包住原始消息(属性带 sender / id / type / time);
//! - `<team-event>` 包住框架事件(`kind` 属性在最前);
//! - `<team-context>` 包住成员的常驻团队状态(自身身份 / 团队元数据);
//! - `<team-note>` 是附加在 inbound / event 块**内部**的最后子元素(它注释的是哪条
//!   消息/事件是树的事实,不是"note 刚好跟在后面");
//! - `for="controller"` 标记投给人类控制者(HITT)的内容。
//!
//! 全部为纯结构函数:调用方传入动态数据 + 已本地化文本(i18n.t),拿回 XML 字符串;
//! `type` / `kind` / `for` 属性值是稳定英文契约 token,永不本地化。
//!
//! 另含快照判定:每类快照事件的每次出现都是某块状态的*完整*快照,最新一次即可代表
//! 更早所有出现;delta(roster-change)或单主体事件(stale-claim 带 task_id)一旦被
//! 丢弃就会丢信息,因此不在集合内。
//!
//! 契约零实现:转义 / 渲染 / 判定算法由插件提供(如 ah-plugins-inbound-render)。

use crate::seam::Seam;

/// `<team-inbound>` 的 `type` 属性契约 token:直发消息。
pub const INBOUND_TYPE_DIRECT: &str = "direct";

/// `<team-inbound>` 的 `type` 属性契约 token:广播消息。
pub const INBOUND_TYPE_BROADCAST: &str = "broadcast";

/// 快照事件种类:每次出现都是某块状态的*完整*快照,最新一次即可代表更早所有出现。
/// 只有具备该性质的 kind 才能进此集合。
pub const SNAPSHOT_EVENT_KINDS: &[&str] = &["task-board"];

/// 本 seam 的服务键(定义于 crate::keys,此处再导出供契约路径使用)。
pub use crate::keys::INBOUND_RENDER;

/// inbound-render Seam(Service Definition):纯函数 XML 渲染。
pub trait InboundRender: Seam {
    /// 渲染一条入站成员/用户消息为 `<team-inbound>` 块。
    #[allow(clippy::too_many_arguments)]
    fn render_inbound(
        &self,
        content: &str,
        sender: &str,
        message_id: &str,
        msg_type: &str,
        time_info: &str,
        for_controller: bool,
        note_kind: Option<&str>,
        note_text: Option<&str>,
    ) -> String;

    /// 渲染一个框架事件为 `<team-event>` 块(`kind` 属性在最前)。
    #[allow(clippy::too_many_arguments)]
    fn render_event(
        &self,
        kind: &str,
        body: &str,
        task_id: Option<&str>,
        for_controller: bool,
        note_kind: Option<&str>,
        note_text: Option<&str>,
    ) -> String;

    /// 渲染常驻团队状态为 `<team-context>` 块。
    fn render_team_context(&self, body: &str) -> String;

    /// 判定一条输入是否纯粹是某类快照事件;是则返回其 kind,否则 None。
    fn snapshot_kind_of(&self, text: &str) -> Option<&'static str>;

    /// 从一批排队输入(旧→新)中剔除被后来者覆盖的快照输入(整条删除),
    /// 非快照输入全部保留、相对顺序不变。
    fn drop_superseded_snapshots(&self, parts: &[String]) -> Vec<String>;
}
