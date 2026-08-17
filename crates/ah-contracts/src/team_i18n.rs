//! team-i18n seam:团队运行时中/英文字符串(对齐 openjiuwen/agent_teams/i18n.py)。
//!
//! 进程级运行时 hard-coded 文案(dispatcher reply-hint、HITT 静默约束等)
//! 按语言切换;长文本模板(prompts/<lang>/*.md)与工具描述(locales/)不走本 seam。
//!
//! 契约零实现:双语表与 `{var}` 占位渲染由插件提供(如 ah-plugins-team-i18n)。

use crate::seam::Seam;

/// 团队运行时语言(对齐 Python `Language = Literal["cn", "en"]`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// 中文。
    Cn,
    /// 英文。
    En,
}

impl Default for Language {
    /// 默认语言:cn(对齐 Python `_DEFAULT_LANGUAGE = "cn"`)。
    fn default() -> Self {
        Language::Cn
    }
}

impl core::fmt::Display for Language {
    /// 语言代码:cn / en(与 serde snake_case 序列化一致)。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Language::Cn => write!(f, "cn"),
            Language::En => write!(f, "en"),
        }
    }
}

/// i18n 解析错误(缺失键等;对齐 Python `KeyError`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I18nError(pub String);

impl core::fmt::Display for I18nError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for I18nError {}

/// user 伪成员名:团队外部真人,不在成员名册里。
pub const USER_PSEUDO_MEMBER_NAME: &str = "user";

/// 通用 reply-hint 文案键(条件版,含 `{sender}` 占位)。
pub const KEY_REPLY_HINT: &str = "dispatcher.reply_hint";
/// user 伪成员专用 reply-hint 文案键(无条件强制版)。
pub const KEY_REPLY_HINT_USER: &str = "dispatcher.reply_hint_user";
/// HITT 静默约束说明文案键。
pub const KEY_HITT_SILENCE_NOTE: &str = "hitt.silence_note";

/// 团队运行时 i18n Seam(Service Definition)。
pub trait TeamI18n: Seam {
    /// 设置当前语言(进程级)。
    fn set_language(&self, lang: Language);

    /// 当前语言。
    fn language(&self) -> Language;

    /// 解析当前语言的文案并渲染 `{var}` 占位。
    ///
    /// 缺失键 → 显式 `Err(I18nError)`;缺失占位参数的行为由实现方决定
    /// (保留原样或 `Err`),必须文档化并测试。
    fn t(&self, key: &str, args: &[(String, String)]) -> Result<String, I18nError>;

    /// 按发件人选 reply-hint 文案:`user` 伪成员走无条件强制版
    /// (`KEY_REPLY_HINT_USER`,无参),其余 sender 走通用条件版
    /// (`KEY_REPLY_HINT`,注入 `{sender}`)。
    fn reply_hint_for(&self, sender: &str) -> String;
}
