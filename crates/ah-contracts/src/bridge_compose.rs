//! bridge_compose seam:bridge avatar 入站/出站文本组装(纯函数)。
//!
//! 对齐 `openjiuwen/agent_teams/agent/bridge_inbound_compose.py` +
//! `bridge_outbound_wrap.py`:
//! - `compose_bridge_inbound`:入站团队消息 + 远程执行结果 → bridge LLM 上下文
//!   (双语模板,明确"仅调度、原样转发"契约);
//! - `wrap_outbound_to_remote`:出站转发文本(PASSTHROUGH 最小头 / REPHRASE
//!   完整发送方上下文 + 任务提示)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现。

use crate::seam::Seam;

/// 远程不可用哨兵(供 compose_bridge_inbound 的 remote_reply 传入)。
pub const REMOTE_UNAVAILABLE_SENTINEL: &str = "__REMOTE_UNAVAILABLE__";

/// 团队成员角色(对齐 agent_teams/schema/team.py TeamRole)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Leader,
    Teammate,
    HumanAgent,
}

impl TeamRole {
    /// 序列化值(对齐 `TeamRole.value`)。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Leader => "leader",
            Self::Teammate => "teammate",
            Self::HumanAgent => "human_agent",
        }
    }
}

/// bridge 邮箱注入模式(对齐 `BridgeMailboxInjectMode`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMailboxInjectMode {
    Passthrough,
    Rephrase,
}

impl BridgeMailboxInjectMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::Rephrase => "rephrase",
        }
    }
}

/// 组装 bridge avatar 的入站上下文文本(对齐 `compose_bridge_inbound`)。
///
/// 双语模板:语言、时间信息可选;标识符(发送者名)不翻译。
pub fn compose_bridge_inbound(
    original_sender: &str,
    original_body: &str,
    remote_reply: &str,
    language: &str,
    time_info: Option<&str>,
) -> String {
    if language == "en" {
        let header = match time_info {
            Some(t) => format!("[Team message from {original_sender} · {t}]"),
            None => format!("[Team message from {original_sender}]"),
        };
        return format!(
            "{header}\n\
             {original_body}\n\n\
             [Remote executor's output — relay this verbatim back to the team]\n\
             {remote_reply}\n\n\
             Your job: schedule only. Decide whether to send_message \
             the remote output above back to {original_sender} verbatim, \
             whether to call claim_task / member_complete_task, or \
             whether to stay silent. Do NOT rewrite or synthesize the \
             remote output — pass it through as-is. The original \
             message has already been forwarded to the remote; do NOT \
             call send_message to forward it again."
        );
    }
    let header = match time_info {
        Some(t) => format!("[来自团队成员 {original_sender} 的消息 · {t}]"),
        None => format!("[来自团队成员 {original_sender} 的消息]"),
    };
    format!(
        "{header}\n\
         {original_body}\n\n\
         [外部执行者的执行结果（要原样回传给团队的内容）]\n\
         {remote_reply}\n\n\
         你的工作：仅做调度。决定是否使用 send_message 把上述执行结果\
         原样回传给 {original_sender}，是否需要调用 claim_task / \
         member_complete_task 等任务管理工具，或保持沉默。\
         **不要改写或综合**执行结果的内容——原样转发即可。\
         注意：原消息已自动转发给外部执行者，无需再调用 send_message 转发原消息。"
    )
}

/// 组装转发给远程 bridge agent 的文本(对齐 `wrap_outbound_to_remote`)。
///
/// 参数镜像 Python 签名(1:1),故允许超 7 参。
#[allow(clippy::too_many_arguments)]
pub fn wrap_outbound_to_remote(
    sender: &str,
    sender_display_name: Option<&str>,
    sender_role: Option<TeamRole>,
    sender_desc: Option<&str>,
    body: &str,
    broadcast: bool,
    task_hint: Option<&str>,
    mode: BridgeMailboxInjectMode,
    language: &str,
) -> String {
    let display = sender_display_name.unwrap_or(sender);
    match mode {
        BridgeMailboxInjectMode::Passthrough => {
            wrap_passthrough(display, body, broadcast, language)
        }
        BridgeMailboxInjectMode::Rephrase => wrap_rephrase(
            display,
            sender_role,
            sender_desc,
            body,
            broadcast,
            task_hint,
            language,
        ),
    }
}

/// PASSTHROUGH:最小发送方头 + 正文。
fn wrap_passthrough(sender_label: &str, body: &str, broadcast: bool, language: &str) -> String {
    if language == "en" {
        let suffix = if broadcast { " (broadcast)" } else { "" };
        format!("[from {sender_label}{suffix}] {body}")
    } else {
        let suffix = if broadcast { "（广播）" } else { "" };
        format!("[来自 {sender_label}{suffix}] {body}")
    }
}

/// REPHRASE:完整发送方上下文 + 正文 + 可选任务提示。
fn wrap_rephrase(
    sender_label: &str,
    sender_role: Option<TeamRole>,
    sender_desc: Option<&str>,
    body: &str,
    broadcast: bool,
    task_hint: Option<&str>,
    language: &str,
) -> String {
    let role_value = sender_role.map(|r| r.as_str()).unwrap_or("unknown");
    let desc = sender_desc.unwrap_or("");
    if language == "en" {
        let kind = if broadcast { "broadcast" } else { "direct" };
        let header =
            format!("[from {sender_label} (role={role_value}, desc={desc:?}, kind={kind})]");
        let suffix = task_hint.map(|h| format!("\nRe: {h}")).unwrap_or_default();
        format!("{header}\n{body}{suffix}")
    } else {
        let kind = if broadcast { "广播" } else { "点对点" };
        let header =
            format!("[来自 {sender_label}（角色={role_value}，描述={desc:?}，类型={kind}）]");
        let suffix = task_hint
            .map(|h| format!("\n相关任务：{h}"))
            .unwrap_or_default();
        format!("{header}\n{body}{suffix}")
    }
}

/// bridge 文本组装 Seam(Service Definition):纯函数门面。
pub trait BridgeCompose: Seam {
    /// 组装入站上下文。
    fn compose_bridge_inbound(
        &self,
        original_sender: &str,
        original_body: &str,
        remote_reply: &str,
        language: &str,
        time_info: Option<&str>,
    ) -> String;

    /// 组装出站转发文本。
    ///
    /// 参数镜像 Python 签名(1:1)。
    #[allow(clippy::too_many_arguments)]
    fn wrap_outbound_to_remote(
        &self,
        sender: &str,
        sender_display_name: Option<&str>,
        sender_role: Option<TeamRole>,
        sender_desc: Option<&str>,
        body: &str,
        broadcast: bool,
        task_hint: Option<&str>,
        mode: BridgeMailboxInjectMode,
        language: &str,
    ) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_cn_includes_header_and_remote() {
        let text = compose_bridge_inbound("alice", "hello", "done", "cn", None);
        assert!(text.contains("[来自团队成员 alice 的消息]"));
        assert!(text.contains("hello"));
        assert!(text.contains("done"));
        assert!(text.contains("仅做调度"));
    }

    #[test]
    fn inbound_en_with_time_info() {
        let text = compose_bridge_inbound("bob", "hi", "ok", "en", Some("10:00 (5m ago)"));
        assert!(text.contains("[Team message from bob · 10:00 (5m ago)]"));
        assert!(text.contains("schedule only"));
    }

    #[test]
    fn passthrough_minimal_header() {
        let text = wrap_outbound_to_remote(
            "alice",
            Some("Alice"),
            None,
            None,
            "hello",
            false,
            None,
            BridgeMailboxInjectMode::Passthrough,
            "cn",
        );
        assert_eq!(text, "[来自 Alice] hello");
        let en = wrap_outbound_to_remote(
            "bob",
            None,
            None,
            None,
            "hi",
            true,
            None,
            BridgeMailboxInjectMode::Passthrough,
            "en",
        );
        assert_eq!(en, "[from bob (broadcast)] hi");
    }

    #[test]
    fn rephrase_full_context() {
        let text = wrap_outbound_to_remote(
            "alice",
            Some("Alice"),
            Some(TeamRole::Teammate),
            Some("frontend dev"),
            "body",
            false,
            Some("task #42: fix bug"),
            BridgeMailboxInjectMode::Rephrase,
            "cn",
        );
        assert!(text.contains("[来自 Alice（角色=teammate，描述=\"frontend dev\"，类型=点对点）]"));
        assert!(text.contains("body"));
        assert!(text.contains("相关任务：task #42: fix bug"));
    }

    #[test]
    fn rephrase_unknown_role_en() {
        let text = wrap_outbound_to_remote(
            "x",
            None,
            None,
            None,
            "b",
            true,
            None,
            BridgeMailboxInjectMode::Rephrase,
            "en",
        );
        assert!(text.contains("role=unknown"));
        assert!(text.contains("kind=broadcast"));
    }

    #[test]
    fn enums_serialize_snake_case() {
        assert_eq!(
            serde_json::to_value(TeamRole::Leader).expect("json"),
            serde_json::json!("leader")
        );
        assert_eq!(
            serde_json::to_value(BridgeMailboxInjectMode::Rephrase).expect("json"),
            serde_json::json!("rephrase")
        );
    }
}
