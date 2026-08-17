//! team-message seam:框架模板团队消息的两阶段渲染(对齐 openjiuwen/agent_teams/message_template.py,F_63)。
//!
//! 发送存意图(meta = {template, refs, params}),投递时按当前任务/成员行渲染:
//! - 占位符契约:单遍替换 `{{ns.field}}`(值永不二次扫描);字段白名单(无 getattr 透传);
//!   未知命名空间/字段/缺失行 → `<missing:ns.field>`(不抛错);
//! - 行缺失(refs 指向的行已不存在)→ RefUnresolved → fallback 行(而不是满篇空洞);
//! - 普通消息(无 meta / meta 畸形 / 无 template 键)原样透传。
//!
//! 契约零实现:解析/替换算法由插件提供(如 ah-plugins-team-message)。

use std::collections::BTreeMap;

use crate::seam::Seam;

/// 模板可读的任务投影(字段白名单,对齐 _TASK_FIELDS)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskView {
    pub task_id: String,
    pub title: String,
    pub content: String,
    pub status: String,
    pub assignee: String,
    pub reviewers: Vec<String>,
    pub review_round: Option<u64>,
    pub max_review_rounds: Option<u64>,
}

/// 模板可读的成员投影(字段白名单,对齐 _MEMBER_FIELDS)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemberView {
    pub member_name: String,
    pub display_name: String,
    pub desc: String,
}

/// 消息 meta(模板描述符)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageMeta {
    pub template: String,
    pub refs: BTreeMap<String, String>,
    pub params: BTreeMap<String, String>,
}

/// 展开结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExpandedMessage {
    pub body: String,
    /// 是否框架模板消息(投递时用于去掉 reply-hint)。
    pub is_template: bool,
}

/// 引用行无法解析(任务被取消清理/成员被移除)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUnresolved(pub String);

impl core::fmt::Display for RefUnresolved {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RefUnresolved {}

/// 团队消息 Seam(Service Definition):meta 解析/构建 + 占位符替换 + 展开。
pub trait TeamMessage: Seam {
    /// 解析 meta 字符串;普通消息(空/畸形/无 template)返回 None。
    fn parse_meta(&self, raw: Option<&str>) -> Option<MessageMeta>;

    /// 组装投递载荷。
    fn build_meta(
        &self,
        template: &str,
        refs: Option<&[(String, String)]>,
        params: Option<&[(String, String)]>,
    ) -> MessageMeta;

    /// 展开失败时的一行替身。
    fn fallback_line(&self, meta: &MessageMeta) -> String;

    /// 单遍替换 `{{ns.field}}`;未知 → `<missing:ns.field>`;值永不二次扫描。
    fn substitute(
        &self,
        template: &str,
        task: Option<&TaskView>,
        member: Option<&MemberView>,
        params: &BTreeMap<String, String>,
    ) -> String;

    /// 展开一条消息:普通消息原样透传;模板消息渲染或降级 fallback。
    fn expand(
        &self,
        content: &str,
        meta_raw: Option<&str>,
        template_content: &str,
        task: Option<&TaskView>,
        member: Option<&MemberView>,
    ) -> ExpandedMessage;
}
