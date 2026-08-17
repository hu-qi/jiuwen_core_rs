//! # ah-plugins-team-i18n
//!
//! 真实团队运行时 i18n(1:1 对齐 openjiuwen/agent_teams/i18n.py):
//! - `Mutex<Language>` 进程级语言状态,默认 cn(对齐 Python `_DEFAULT_LANGUAGE`);
//! - 内置双语短串表(至少含 dispatcher.reply_hint / dispatcher.reply_hint_user /
//!   hitt.silence_note 三键 + 测试用补充键),cn/en 双语;
//! - `t()`:查表 + 手写 `{var}` 占位渲染(不依赖 regex;对齐 format_map 的
//!   `{name}` 形态,不支持格式说明符)。缺失键 → 显式 `I18nError`;
//!   缺失占位参数 → **保留原样**(显式降级,见 `render` 文档);
//! - `reply_hint_for(sender)`:`sender == "user"` → 无条件强制版
//!   (dispatcher.reply_hint_user,无参);其余 sender → 通用条件版
//!   (dispatcher.reply_hint,注入 `{sender}`)。

use std::sync::{Arc, Mutex};

use ah_contracts::keys::TEAM_I18N;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_i18n::{
    I18nError, KEY_HITT_SILENCE_NOTE, KEY_REPLY_HINT, KEY_REPLY_HINT_USER, Language, TeamI18n,
    USER_PSEUDO_MEMBER_NAME,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 键 → (cn, en) 双语短串表(对齐 i18n.py STRINGS 的 cn / en 两表)。
///
/// 只装运行时 hard-coded 短串;长文本模板(prompts/<lang>/*.md)与工具描述
/// (locales/)不进本表。新增键在此登记即可;表内键对 `t()` 保证存在。
const STRINGS: &[(&str, (&str, &str))] = &[
    // agent/dispatcher.py — reply hints(条件版 / user 强制版)
    (
        KEY_REPLY_HINT,
        (
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 {sender}。",
            "If the sender is asking or waiting for a reply, be sure to reply to {sender} via send_message.",
        ),
    ),
    (
        KEY_REPLY_HINT_USER,
        (
            "这条消息来自 user——委托本团队工作的**团队外部真人**，不是团队成员，不在成员名册里。你必须调用 send_message(to=\"user\") 把答复发回用户；你写在回复正文里的任何文字都不会送达用户，不调用工具就等于没有回复。",
            "This message is from user — the **human outside the team** who commissioned this team's work. They are not a team member and do not appear in the roster. You MUST call send_message(to=\"user\") to deliver your reply; any text you write in your reply body never reaches the user, so not calling the tool means you did not reply at all.",
        ),
    ),
    // XML inbound track — HITT 静默约束(3 行,load-bearing)
    (
        KEY_HITT_SILENCE_NOTE,
        (
            "**这是给控制者看的通知，不是要你执行的指令**，运行时已把它原样转给控制者。\n**严格禁止任何自主行为**：禁止主动回复发送方 / 指派方（包括调用 send_message）、禁止自主调用 member_complete_task / claim_task / 文件 / shell 等任何工具去回应或推进、禁止用纯文本输出表达意图或承诺。\n**保持静默**，只有控制者在 Inbox 里下达明确指令后才能行动。",
            "**This is a notification for your controller, NOT an instruction for you to act on**; the runtime has already surfaced it to the controller as-is.\n**Autonomous behavior is strictly forbidden**: do not reply to the sender / assigner (including via send_message), do not autonomously call member_complete_task / claim_task / file tools / shell tools or any other tool to respond or push work forward, and do not emit plain-text intent or promises.\n**Stay silent** and act only after the controller issues an explicit instruction via the Inbox.",
        ),
    ),
    // agent/dispatcher.py — 成员事件(测试用补充键)
    (
        "dispatcher.member_online",
        (
            "[成员事件] 成员 {target_id} 已上线",
            "[Member Event] Member {target_id} is online",
        ),
    ),
    // timefmt.py — 未知时间(测试用补充键,无占位)
    ("time.unknown", ("时间未知", "unknown time")),
];

/// 手写 `{var}` 占位渲染(不依赖 regex)。
///
/// 行为契约(对齐 Python `str.format_map` 的 `{name}` 形态,但不支持格式
/// 说明符如 `{value:02d}`):
/// - 参数命中 → 替换为参数值;
/// - **缺失占位参数 → 保留原样**(显式降级:输出保留 `{var}` 字面量),
///   调用方漏参时整条文案不报废,文案也可携带字面花括号;
/// - 多余参数 → 忽略(format_map 语义);
/// - 未闭合的 `{` 与空名 `{}` → 原样保留。
fn render(template: &str, args: &[(String, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find('}') {
            let name = &after_open[..close];
            if !name.is_empty()
                && let Some((_, value)) = args.iter().find(|(k, _)| k.as_str() == name)
            {
                out.push_str(value);
                rest = &after_open[close + 1..];
                continue;
            }
        }
        // 未闭合 / 空名 / 缺失参数 → 保留 `{` 字面量并继续扫描。
        out.push('{');
        rest = &rest[open + 1..];
    }
    out.push_str(rest);
    out
}

/// 真实 team-i18n 实现:进程级语言状态 + 内置双语短串表。
pub struct TeamI18nService {
    language: Mutex<Language>,
}

impl TeamI18nService {
    /// 默认实例:语言 = cn(对齐 Python `_DEFAULT_LANGUAGE`)。
    pub fn new() -> Self {
        Self {
            language: Mutex::new(Language::Cn),
        }
    }
}

impl Default for TeamI18nService {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for TeamI18nService {}

impl TeamI18n for TeamI18nService {
    fn set_language(&self, lang: Language) {
        let mut guard = self
            .language
            .lock()
            .expect("team-i18n language mutex poisoned");
        *guard = lang;
    }

    fn language(&self) -> Language {
        *self
            .language
            .lock()
            .expect("team-i18n language mutex poisoned")
    }

    fn t(&self, key: &str, args: &[(String, String)]) -> Result<String, I18nError> {
        let lang = self.language();
        let raw = STRINGS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, pair)| pair)
            .ok_or_else(|| I18nError(format!("Missing i18n key '{key}' for language '{lang}'")))?;
        let template = match lang {
            Language::Cn => raw.0,
            Language::En => raw.1,
        };
        Ok(render(template, args))
    }

    fn reply_hint_for(&self, sender: &str) -> String {
        if sender == USER_PSEUDO_MEMBER_NAME {
            // 内置键编译期保证存在;缺失即编程错误(显式 panic,不静默)。
            self.t(KEY_REPLY_HINT_USER, &[])
                .expect("builtin key dispatcher.reply_hint_user must exist")
        } else {
            let args = [(String::from("sender"), sender.to_string())];
            self.t(KEY_REPLY_HINT, &args)
                .expect("builtin key dispatcher.reply_hint must exist")
        }
    }
}

/// team-i18n 插件:注册 `team-i18n` seam。
pub struct TeamI18nPlugin;

impl Plugin for TeamI18nPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-i18n"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_I18N]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let i18n: Arc<dyn TeamI18n> = Arc::new(TeamI18nService::new());
        Ok(vec![ctx.register(TEAM_I18N, i18n)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;

    fn kv(key: &str, value: &str) -> (String, String) {
        (key.to_string(), value.to_string())
    }

    #[test]
    fn t_lookup_bilingual() {
        let svc = TeamI18nService::new();
        // 默认 cn。
        assert_eq!(svc.language(), Language::Cn);
        assert_eq!(svc.t("time.unknown", &[]).unwrap(), "时间未知");
        assert_eq!(
            svc.t("dispatcher.member_online", &[kv("target_id", "dev-1")])
                .unwrap(),
            "[成员事件] 成员 dev-1 已上线"
        );
        // 切 en。
        svc.set_language(Language::En);
        assert_eq!(svc.t("time.unknown", &[]).unwrap(), "unknown time");
        assert_eq!(
            svc.t("dispatcher.member_online", &[kv("target_id", "dev-1")])
                .unwrap(),
            "[Member Event] Member dev-1 is online"
        );
    }

    #[test]
    fn t_renders_placeholders() {
        let svc = TeamI18nService::new();
        assert_eq!(
            svc.t(KEY_REPLY_HINT, &[kv("sender", "dev-1")]).unwrap(),
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 dev-1。"
        );
    }

    #[test]
    fn t_missing_key_errs_with_language() {
        let svc = TeamI18nService::new();
        let err = svc.t("no.such.key", &[]).unwrap_err();
        assert!(err.0.contains("no.such.key"));
        assert!(err.0.contains("'cn'"));
        svc.set_language(Language::En);
        let err2 = svc.t("no.such.key", &[]).unwrap_err();
        assert!(err2.0.contains("'en'"));
        // 空键同样显式报错,不静默。
        assert!(svc.t("", &[]).is_err());
    }

    #[test]
    fn t_missing_placeholder_arg_kept_as_is() {
        let svc = TeamI18nService::new();
        // 显式降级:缺失参数保留 {sender} 字面量。
        assert_eq!(
            svc.t(KEY_REPLY_HINT, &[]).unwrap(),
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 {sender}。"
        );
        // 参数键不匹配 → 同样保留。
        assert_eq!(
            svc.t(KEY_REPLY_HINT, &[kv("other", "x")]).unwrap(),
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 {sender}。"
        );
    }

    #[test]
    fn t_extra_args_ignored() {
        let svc = TeamI18nService::new();
        let rendered = svc
            .t(KEY_REPLY_HINT, &[kv("sender", "dev-9"), kv("extra", "zzz")])
            .unwrap();
        assert_eq!(
            rendered,
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 dev-9。"
        );
        assert!(!rendered.contains("zzz"));
    }

    #[test]
    fn reply_hint_for_user_forced_variant() {
        let svc = TeamI18nService::new();
        let hint = svc.reply_hint_for(USER_PSEUDO_MEMBER_NAME);
        // 与 t(KEY_REPLY_HINT_USER) 无参渲染逐字一致。
        assert_eq!(hint, svc.t(KEY_REPLY_HINT_USER, &[]).unwrap());
        assert!(hint.contains("send_message(to=\"user\")"));
        assert!(hint.contains("团队外部真人"));
        // 强制版无占位。
        assert!(!hint.contains("{sender}"));
    }

    #[test]
    fn reply_hint_for_other_sender_generic() {
        let svc = TeamI18nService::new();
        let hint = svc.reply_hint_for("dev-1");
        assert_eq!(
            hint,
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 dev-1。"
        );
        assert_eq!(
            hint,
            svc.t(KEY_REPLY_HINT, &[kv("sender", "dev-1")]).unwrap()
        );
        // 空 sender 不是 user 伪成员 → 走通用版(占位渲染为空串)。
        assert_eq!(
            svc.reply_hint_for(""),
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 。"
        );
        // user 匹配精确区分大小写。
        assert_eq!(
            svc.reply_hint_for("User"),
            "如果对方在提问或等待回复，请务必通过 send_message 工具回复 User。"
        );
    }

    #[test]
    fn set_language_switches_t_and_reply_hint() {
        let svc = TeamI18nService::new();
        assert_eq!(svc.language(), Language::Cn);
        assert!(
            svc.t(KEY_REPLY_HINT, &[kv("sender", "dev-1")])
                .unwrap()
                .contains("请务必")
        );
        svc.set_language(Language::En);
        assert_eq!(svc.language(), Language::En);
        assert_eq!(
            svc.reply_hint_for("dev-1"),
            "If the sender is asking or waiting for a reply, be sure to reply to dev-1 via send_message."
        );
        // 切回 cn。
        svc.set_language(Language::Cn);
        assert_eq!(svc.language(), Language::Cn);
        assert!(svc.reply_hint_for("dev-1").contains("请务必"));
    }

    #[test]
    fn silence_note_bilingual() {
        let svc = TeamI18nService::new();
        let cn = svc.t(KEY_HITT_SILENCE_NOTE, &[]).unwrap();
        assert!(cn.contains("严格禁止任何自主行为"));
        assert!(cn.contains("保持静默"));
        assert!(cn.contains('\n'));
        assert_eq!(cn.lines().count(), 3, "cn 静默说明应为 3 行");
        svc.set_language(Language::En);
        let en = svc.t(KEY_HITT_SILENCE_NOTE, &[]).unwrap();
        assert!(en.contains("Autonomous behavior is strictly forbidden"));
        assert!(en.contains("Stay silent"));
        assert!(en.contains('\n'));
        assert_eq!(en.lines().count(), 3, "en 静默说明应为 3 行");
    }

    #[test]
    fn error_display_and_language_serde() {
        // I18nError Display + Error 契约。
        let err = I18nError("Missing i18n key 'x' for language 'cn'".to_string());
        assert_eq!(err.to_string(), "Missing i18n key 'x' for language 'cn'");
        let _: &dyn std::error::Error = &err;
        // Language serde snake_case 契约:cn / en,非法值报错。
        assert_eq!(serde_json::to_string(&Language::Cn).unwrap(), "\"cn\"");
        assert_eq!(serde_json::to_string(&Language::En).unwrap(), "\"en\"");
        assert_eq!(
            serde_json::from_str::<Language>("\"cn\"").unwrap(),
            Language::Cn
        );
        assert!(serde_json::from_str::<Language>("\"zh\"").is_err());
        assert_eq!(Language::Cn.to_string(), "cn");
        assert_eq!(Language::En.to_string(), "en");
    }

    #[test]
    fn plugin_registers_team_i18n() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamI18nPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let i18n = ctx
            .service::<dyn TeamI18n>(&TEAM_I18N)
            .expect("team-i18n seam");
        // 默认 cn,经 seam 可解析。
        assert_eq!(i18n.language(), Language::Cn);
        assert!(
            i18n.t(KEY_REPLY_HINT, &[kv("sender", "dev-1")])
                .unwrap()
                .contains("请务必")
        );
        // user 强制版经 seam 可用。
        assert_eq!(
            i18n.reply_hint_for("user"),
            i18n.t(KEY_REPLY_HINT_USER, &[]).unwrap()
        );
        // 可逆注册:drop effects 后 seam 消失。
        drop(effects);
        assert!(!ctx.has_service(&TEAM_I18N));
    }
}
