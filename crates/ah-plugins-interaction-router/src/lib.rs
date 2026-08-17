//! # ah-plugins-interaction-router
//!
//! 真实 interact-str 语法解析(1:1 对齐 openjiuwen/agent_teams/interaction/router.py):
//! "# " god-view / "$<name>" human-agent 频道 / "@<member>" 点对点 / "@all|@*" 广播 /
//! 多收件人 fan-out,以及未知 @mention 折叠回无定向消息(保留原文)。
//!
//! 纯函数、无 IO、无状态。三处正则(parse_mention / $ 前缀 / 收件人循环)以无依赖的
//! 字符扫描实现(插件 Cargo.toml 只允许 ah-hub + ah-contracts + serde_json,不引入 regex)。
//! 空白判定与 Python str 正则的 \s 对齐:char::is_whitespace(ASCII 空白 + Unicode 空白)。

use std::sync::Arc;

use ah_contracts::interaction_router::{
    BROADCAST_TARGETS, GOD_VIEW_PREFIX, InteractPayload, InteractionRouter, PayloadKind,
    RESERVED_MEMBER_NAMES, USER_PSEUDO_MEMBER_NAME,
};
use ah_contracts::keys::INTERACTION_ROUTER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实交互路由实现(纯语法解析)。
pub struct InteractionRouterService;

impl Seam for InteractionRouterService {}

/// 从 `from` 起连续消费满足 pred 的字符,返回结束字节偏移(必为字符边界)。
/// 等价于正则字符类 `[...]+` 的一次贪心匹配;`from` 必须落在字符边界上。
fn run_until(rest: &str, from: usize, pred: impl Fn(char) -> bool) -> usize {
    let mut end = from;
    for (idx, ch) in rest[from..].char_indices() {
        if !pred(ch) {
            break;
        }
        end = from + idx + ch.len_utf8();
    }
    end
}

/// `^@(\S+)\s+` → (target, 剩余文本);不匹配返回 None。
/// \S+ 以整字符消费(多字节字符不会被空白字节截断),切片必然落在字符边界。
fn take_recipient(rest: &str) -> Option<(String, &str)> {
    if !rest.starts_with('@') {
        return None;
    }
    let name_end = run_until(rest, 1, |c| !c.is_whitespace());
    if name_end == 1 {
        return None;
    }
    let ws_end = run_until(rest, name_end, char::is_whitespace);
    if ws_end == name_end {
        return None;
    }
    Some((rest[1..name_end].to_string(), &rest[ws_end..]))
}

/// `^\$([^\s@]+)(?:\s+|(?=@))([\s\S]*)$` → (name, 剩余文本);不匹配返回 None。
fn take_human_agent_prefix(rest: &str) -> Option<(String, &str)> {
    if !rest.starts_with('$') {
        return None;
    }
    let name_end = run_until(rest, 1, |c| !c.is_whitespace() && c != '@');
    if name_end == 1 || name_end >= rest.len() {
        // [^\s@]+ 为空,或名字已到串尾(\s+ 与 (?=@) 都失败)。
        return None;
    }
    // (?=@):零宽前瞻,剩余文本从 @ 处开始($avatar@member hello)。
    if rest[name_end..].starts_with('@') {
        return Some((rest[1..name_end].to_string(), &rest[name_end..]));
    }
    // \s+ 分隔符。
    let ws_end = run_until(rest, name_end, char::is_whitespace);
    if ws_end == name_end {
        return None;
    }
    Some((rest[1..name_end].to_string(), &rest[ws_end..]))
}

/// `^@(\S+)\s+([\s\S]+)$` — 单个 @target body 提及。
fn parse_mention_impl(content: &str) -> Option<(String, String)> {
    let (target, rest) = take_recipient(content)?;
    if rest.is_empty() {
        return None; // ([sS]+) 至少需要一个字符
    }
    Some((target, rest.to_string()))
}

/// 把自由文本 interact 体翻译为类型化载荷列表;空 / 纯空白输入返回空列表。
fn parse_interact_str_impl(body: &str) -> Vec<InteractPayload> {
    if body.trim().is_empty() {
        return Vec::new();
    }

    let mut rest = body;
    let mut sender = USER_PSEUDO_MEMBER_NAME.to_string();
    let mut is_human_agent = false;

    // ---- 频道前缀 --------------------------------------------------
    if rest.starts_with(GOD_VIEW_PREFIX) {
        rest = rest[GOD_VIEW_PREFIX.len()..].trim_start();
    } else if let Some((name, after)) = take_human_agent_prefix(rest) {
        sender = name;
        rest = after.trim_start();
        is_human_agent = true;
    }
    // 无识别前缀 → 按 "# " 缺省频道处理,rest 保持原始 body。

    // ---- 收件人 ----------------------------------------------------
    let mut recipients: Vec<String> = Vec::new();
    while let Some((name, after)) = take_recipient(rest) {
        recipients.push(name);
        rest = after;
    }

    let final_body = rest.to_string();

    // ---- 载荷合成 --------------------------------------------------
    if recipients.is_empty() {
        return if is_human_agent {
            vec![InteractPayload::human_agent(final_body, sender, None)]
        } else {
            vec![InteractPayload::god_view(final_body)]
        };
    }

    let has_broadcast = recipients
        .iter()
        .any(|r| BROADCAST_TARGETS.contains(&r.as_str()));
    if has_broadcast {
        // 广播覆盖其余列名收件人——广播已覆盖每个成员。
        return if is_human_agent {
            vec![InteractPayload::human_agent(
                final_body,
                sender,
                Some("*".to_string()),
            )]
        } else {
            vec![InteractPayload::operator(final_body, None)]
        };
    }

    if is_human_agent {
        recipients
            .into_iter()
            .map(|name| {
                InteractPayload::human_agent(final_body.clone(), sender.clone(), Some(name))
            })
            .collect()
    } else {
        recipients
            .into_iter()
            .map(|name| InteractPayload::operator(final_body.clone(), Some(name)))
            .collect()
    }
}

/// 点对点收件人;god-view / avatar-drive / 广播(含 @all、@*)均无定向,返回 None。
fn named_target(payload: &InteractPayload) -> Option<&str> {
    match payload.kind {
        PayloadKind::GodView => None,
        PayloadKind::Operator | PayloadKind::HumanAgent => match payload.target.as_deref() {
            Some(t) if !BROADCAST_TARGETS.contains(&t) => Some(t),
            _ => None,
        },
    }
}

/// 把未知 @mention 载荷折叠回一条无定向消息:提及拼回正文,原样保留用户文本。
/// 样本为 HumanAgent → HumanAgent(保留 sender);否则 → GodView。
fn fold_unknown_mentions(unknown: &[InteractPayload]) -> InteractPayload {
    let sample = &unknown[0];
    let mentions = unknown
        .iter()
        .filter_map(|p| p.target.as_deref().map(|t| format!("@{}", t)))
        .collect::<Vec<_>>()
        .join(" ");
    let general_body = if sample.body.is_empty() {
        mentions
    } else {
        format!("{} {}", mentions, sample.body)
    };
    match sample.kind {
        PayloadKind::HumanAgent => {
            InteractPayload::human_agent(general_body, sample.sender.clone(), None)
        }
        _ => InteractPayload::god_view(general_body),
    }
}

/// 同步成员解析:点对点 target 不在名册 → 折叠;全部 known → 原样返回。
fn resolve_targets_impl(
    payloads: &[InteractPayload],
    member_exists: &dyn Fn(&str) -> bool,
) -> Vec<InteractPayload> {
    let mut kept: Vec<InteractPayload> = Vec::new();
    let mut unknown: Vec<InteractPayload> = Vec::new();
    for payload in payloads {
        match named_target(payload) {
            Some(name) if !member_exists(name) => unknown.push(payload.clone()),
            _ => kept.push(payload.clone()),
        }
    }
    if unknown.is_empty() {
        return payloads.to_vec();
    }
    kept.push(fold_unknown_mentions(&unknown));
    kept
}

impl InteractionRouter for InteractionRouterService {
    fn parse_mention(&self, content: &str) -> Option<(String, String)> {
        parse_mention_impl(content)
    }

    fn is_reserved_name(&self, name: &str) -> bool {
        RESERVED_MEMBER_NAMES.contains(&name)
    }

    fn parse_interact_str(&self, body: &str) -> Vec<InteractPayload> {
        parse_interact_str_impl(body)
    }

    fn resolve_targets(
        &self,
        payloads: &[InteractPayload],
        member_exists: &dyn Fn(&str) -> bool,
    ) -> Vec<InteractPayload> {
        resolve_targets_impl(payloads, member_exists)
    }
}

/// interaction-router 插件:注册 `interaction-router` seam。
pub struct InteractionRouterPlugin;

impl Plugin for InteractionRouterPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-interaction-router"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![INTERACTION_ROUTER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let router: Arc<dyn InteractionRouter> = Arc::new(InteractionRouterService);
        Ok(vec![ctx.register(INTERACTION_ROUTER, router)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;

    fn router() -> InteractionRouterService {
        InteractionRouterService
    }

    #[test]
    fn parse_mention_matches_target_and_body() {
        let r = router();
        assert_eq!(
            r.parse_mention("@alice hello world"),
            Some(("alice".to_string(), "hello world".to_string()))
        );
        // 多个空白分隔符也被吃掉(\s+)。
        assert_eq!(
            r.parse_mention("@bob  double space"),
            Some(("bob".to_string(), "double space".to_string()))
        );
    }

    #[test]
    fn parse_mention_returns_none_without_mention_prefix() {
        let r = router();
        assert_eq!(r.parse_mention(""), None);
        assert_eq!(r.parse_mention("plain text"), None);
        assert_eq!(r.parse_mention("hello @alice"), None); // 非前缀位置
        assert_eq!(r.parse_mention("@alice"), None); // 缺 body
        assert_eq!(r.parse_mention("@ alice"), None); // 缺 target
    }

    #[test]
    fn reserved_names_are_recognized() {
        let r = router();
        for name in ["user", "team_leader", "human_agent"] {
            assert!(r.is_reserved_name(name), "{name} 应为保留名");
        }
        assert!(!r.is_reserved_name("alice"));
        assert!(!r.is_reserved_name(""));
    }

    #[test]
    fn god_view_channel_and_default_prefix() {
        let r = router();
        // "# " 前缀吃掉后 lstrip。
        assert_eq!(
            r.parse_interact_str("# hello world"),
            vec![InteractPayload::god_view("hello world")]
        );
        // 无前缀 → 缺省 "# " 频道,body 保持原样。
        assert_eq!(
            r.parse_interact_str("hello world"),
            vec![InteractPayload::god_view("hello world")]
        );
        // #hashtag 无尾随空格 → 非频道,整串按正文。
        assert_eq!(
            r.parse_interact_str("#hashtag content"),
            vec![InteractPayload::god_view("#hashtag content")]
        );
    }

    #[test]
    fn human_agent_channel_avatar_drive() {
        let r = router();
        // $<name> 无收件人 → avatar-drive,target None,sender 为频道名。
        assert_eq!(
            r.parse_interact_str("$avatar drive now"),
            vec![InteractPayload::human_agent("drive now", "avatar", None)]
        );
        // $<name>@member 无空白分隔也可(前瞻 @)。
        assert_eq!(
            r.parse_interact_str("$avatar@alice hello"),
            vec![InteractPayload::human_agent(
                "hello",
                "avatar",
                Some("alice".to_string())
            )]
        );
    }

    #[test]
    fn directed_operator_and_human_agent() {
        let r = router();
        // "# " + @member → Operator 点对点。
        assert_eq!(
            r.parse_interact_str("# @alice hello"),
            vec![InteractPayload::operator(
                "hello",
                Some("alice".to_string())
            )]
        );
        // 无频道前缀的 @member → 缺省 "# " → Operator。
        assert_eq!(
            r.parse_interact_str("@alice hello"),
            vec![InteractPayload::operator(
                "hello",
                Some("alice".to_string())
            )]
        );
        // $ + @member → HumanAgent 点对点,sender 为频道名。
        assert_eq!(
            r.parse_interact_str("$avatar @alice hello"),
            vec![InteractPayload::human_agent(
                "hello",
                "avatar",
                Some("alice".to_string())
            )]
        );
    }

    #[test]
    fn broadcast_overrides_other_recipients() {
        let r = router();
        // @all → Operator 广播(target None),其余列名收件人忽略。
        assert_eq!(
            r.parse_interact_str("@all everyone"),
            vec![InteractPayload::operator("everyone", None)]
        );
        assert_eq!(
            r.parse_interact_str("@* hi there"),
            vec![InteractPayload::operator("hi there", None)]
        );
        assert_eq!(
            r.parse_interact_str("@all @bob everyone"),
            vec![InteractPayload::operator("everyone", None)]
        );
        // $ + @all → HumanAgent 广播(target="*")。
        assert_eq!(
            r.parse_interact_str("$avatar @all hi"),
            vec![InteractPayload::human_agent(
                "hi",
                "avatar",
                Some("*".to_string())
            )]
        );
    }

    #[test]
    fn multi_cast_fan_out_preserves_order() {
        let r = router();
        // 每个收件人一条,顺序保持。
        assert_eq!(
            r.parse_interact_str("@m1 @m2 hello"),
            vec![
                InteractPayload::operator("hello", Some("m1".to_string())),
                InteractPayload::operator("hello", Some("m2".to_string())),
            ]
        );
        assert_eq!(
            r.parse_interact_str("$avatar @m1 @m2 hi"),
            vec![
                InteractPayload::human_agent("hi", "avatar", Some("m1".to_string())),
                InteractPayload::human_agent("hi", "avatar", Some("m2".to_string())),
            ]
        );
    }

    #[test]
    fn empty_or_whitespace_input_yields_no_payloads() {
        let r = router();
        assert_eq!(r.parse_interact_str(""), Vec::<InteractPayload>::new());
        assert_eq!(
            r.parse_interact_str("   \t "),
            Vec::<InteractPayload>::new()
        );
    }

    #[test]
    fn resolve_targets_keeps_known_payloads_unchanged() {
        let r = router();
        let payloads = vec![
            InteractPayload::god_view("status"),
            InteractPayload::operator("hi", Some("alice".to_string())),
            InteractPayload::operator("broadcast", None),
        ];
        let roster = |name: &str| name == "alice";
        let resolved = r.resolve_targets(&payloads, &roster);
        assert_eq!(resolved, payloads);
    }

    #[test]
    fn resolve_targets_folds_unknown_mention_into_god_view() {
        let r = router();
        let payloads = vec![InteractPayload::operator(
            "hello",
            Some("ghost".to_string()),
        )];
        let roster = |_name: &str| false;
        let resolved = r.resolve_targets(&payloads, &roster);
        assert_eq!(resolved, vec![InteractPayload::god_view("@ghost hello")]);
    }

    #[test]
    fn resolve_targets_mixed_known_and_unknown() {
        let r = router();
        let payloads = vec![
            InteractPayload::operator("one", Some("alice".to_string())),
            InteractPayload::operator("two", Some("ghost".to_string())),
            InteractPayload::operator("three", Some("bob".to_string())),
        ];
        let roster = |name: &str| name == "alice" || name == "bob";
        let resolved = r.resolve_targets(&payloads, &roster);
        // kept 保持输入顺序,unknown 折叠为一条 GodView 追加在尾部。
        assert_eq!(
            resolved,
            vec![
                InteractPayload::operator("one", Some("alice".to_string())),
                InteractPayload::operator("three", Some("bob".to_string())),
                InteractPayload::god_view("@ghost two"),
            ]
        );
    }

    #[test]
    fn resolve_targets_human_agent_fold_keeps_sender() {
        let r = router();
        let payloads = vec![InteractPayload::human_agent(
            "hello",
            "avatar",
            Some("ghost".to_string()),
        )];
        let roster = |_name: &str| false;
        let resolved = r.resolve_targets(&payloads, &roster);
        assert_eq!(
            resolved,
            vec![InteractPayload::human_agent("@ghost hello", "avatar", None)]
        );
    }

    #[test]
    fn resolve_targets_folds_empty_body_to_mentions_only() {
        let r = router();
        let payloads = vec![
            InteractPayload::operator("", Some("ghost1".to_string())),
            InteractPayload::operator("", Some("ghost2".to_string())),
        ];
        let roster = |_name: &str| false;
        let resolved = r.resolve_targets(&payloads, &roster);
        assert_eq!(resolved, vec![InteractPayload::god_view("@ghost1 @ghost2")]);
    }

    #[test]
    fn broadcast_and_god_view_pass_through_resolve() {
        let r = router();
        // 无定向载荷(god-view / 广播 / avatar-drive)即使名册全空也原样通过。
        let payloads = vec![
            InteractPayload::god_view("status"),
            InteractPayload::operator("all hands", None),
            InteractPayload::operator("team", Some("*".to_string())),
            InteractPayload::human_agent("drive", "avatar", Some("*".to_string())),
        ];
        let roster = |_name: &str| false;
        let resolved = r.resolve_targets(&payloads, &roster);
        assert_eq!(resolved, payloads);
    }

    #[test]
    fn plugin_registers_interaction_router() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(InteractionRouterPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let router = ctx
            .service::<dyn InteractionRouter>(&INTERACTION_ROUTER)
            .expect("interaction-router seam");
        assert!(router.is_reserved_name("user"));
        assert_eq!(
            router.parse_mention("@alice hi"),
            Some(("alice".to_string(), "hi".to_string()))
        );
        assert_eq!(
            router.parse_interact_str("# hello"),
            vec![InteractPayload::god_view("hello")]
        );
        drop(effects);
        assert!(!ctx.has_service(&INTERACTION_ROUTER));
    }
}
