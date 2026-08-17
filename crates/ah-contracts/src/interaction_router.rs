//! # interaction-router 契约
//!
//! 交互输入语法解析的 seam(1:1 对齐 openjiuwen/agent_teams/interaction/router.py)。
//! 本模块只声明类型 / trait / 常量,零服务实现;语法解析实现在
//! ah-plugins-interaction-router 插件中。
//!
//! 语法总览:
//! - 输入 := channel? recipients? body;
//! - channel := "# " | "$" name (" " | "@"),缺省 "# ";
//! - recipients := ("@" name " ")*;
//! - "#hashtag" / "$variable" 因缺少尾随空格,是正文而非频道标记。

use crate::seam::Seam;

/// 交互视角种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    /// God 视角:直接对队长 DeepAgent 说话(等价历史 invoke 频道)。
    GodView,
    /// Operator 视角:以外部用户身份说话(@member 点对点 / @all 广播)。
    Operator,
    /// HumanAgent 视角:以已注册 human-agent 团队成员身份说话。
    HumanAgent,
}

/// 一次 interact(str, ...) 解析出的类型化载荷。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractPayload {
    /// 交互视角。
    pub kind: PayloadKind,
    /// 消息内容。
    pub body: String,
    /// 发送者名字;缺省为 USER_PSEUDO_MEMBER_NAME("user")。
    pub sender: String,
    /// 点对点收件人;None 表示无定向(god-view / avatar-drive / 广播)。
    pub target: Option<String>,
}

impl InteractPayload {
    /// God-view 载荷:body 直达队长 DeepAgent,sender 为 "user"。
    pub fn god_view(body: impl Into<String>) -> Self {
        Self {
            kind: PayloadKind::GodView,
            body: body.into(),
            sender: USER_PSEUDO_MEMBER_NAME.to_string(),
            target: None,
        }
    }

    /// Operator 载荷:sender 为 "user";target None 表示广播到全队。
    pub fn operator(body: impl Into<String>, target: Option<String>) -> Self {
        Self {
            kind: PayloadKind::Operator,
            body: body.into(),
            sender: USER_PSEUDO_MEMBER_NAME.to_string(),
            target,
        }
    }

    /// HumanAgent 载荷:以 human-agent 成员 sender 身份说话。
    pub fn human_agent(
        body: impl Into<String>,
        sender: impl Into<String>,
        target: Option<String>,
    ) -> Self {
        Self {
            kind: PayloadKind::HumanAgent,
            body: body.into(),
            sender: sender.into(),
            target,
        }
    }
}

/// 单次载荷投递结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverResult {
    /// 是否投递成功。
    pub ok: bool,
    /// 成功时携带的消息 id;失败时为空字符串。
    pub message_id: String,
    /// 失败时的短稳定原因 token(如 unknown_member:<target>);成功时为空字符串。
    pub reason: String,
}

impl DeliverResult {
    /// 成功结果,可选携带消息 id。
    pub fn success(message_id: impl Into<String>) -> Self {
        Self {
            ok: true,
            message_id: message_id.into(),
            reason: String::new(),
        }
    }

    /// 失败结果,携带短原因 token。
    pub fn failure(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            message_id: String::new(),
            reason: reason.into(),
        }
    }
}

/// 外部调用者的伪成员名(非真实团队成员)。
pub const USER_PSEUDO_MEMBER_NAME: &str = "user";

/// 运行时保留成员名:user 声明的成员不可占用这些名字。
pub const RESERVED_MEMBER_NAMES: &[&str] = &["user", "team_leader", "human_agent"];

/// 保留广播目标(@all / @* 表示全队广播)。
pub const BROADCAST_TARGETS: &[&str] = &["all", "*"];

/// god-view 频道前缀;必须带尾随空格,"#hashtag" 不是频道标记。
pub const GOD_VIEW_PREFIX: &str = "# ";

/// 交互路由 seam:interact 文本 → 类型化载荷的纯语法解析。
///
/// 解析不接触成员表(纯语法);成员存在性校验由 resolve_targets 的调用方
/// 谓词闭包提供。
pub trait InteractionRouter: Seam {
    /// 解析单个 "@target body" 提及;空输入 / 无提及前缀时返回 None。
    fn parse_mention(&self, content: &str) -> Option<(String, String)>;

    /// name 是否命中运行时保留成员名。
    fn is_reserved_name(&self, name: &str) -> bool;

    /// 把自由文本 interact 体翻译为类型化载荷列表;空 / 纯空白输入返回空列表。
    fn parse_interact_str(&self, body: &str) -> Vec<InteractPayload>;

    /// 同步版成员解析:点对点 target 不在名册时折叠回无定向消息并保留原文。
    fn resolve_targets(
        &self,
        payloads: &[InteractPayload],
        member_exists: &dyn Fn(&str) -> bool,
    ) -> Vec<InteractPayload>;
}
