//! roster-diff seam:团队成员名册 diff 与正文渲染(对齐 openjiuwen/agent_teams/prompts/messages.py)。
//!
//! - `diff_roster`:按 member_name 键对比新旧名册,输出 joined/left/changed;
//!   仅跟踪 display_name/desc/role 三个字段(运行时状态不跟踪);
//! - `format_member_line`:一行名册条目(可带 [human] 标记与 [prefix] 前缀);
//! - 正文:全量快照(首次)与增量(变更)两种渲染,双语(cn/en)标签。
//!
//! 契约零实现:diff 与渲染算法由插件提供(如 ah-plugins-roster-diff)。

use crate::seam::Seam;

/// 名册成员(对齐 dict[str,str] 的成员映射)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RosterMember {
    pub member_name: String,
    pub display_name: String,
    pub desc: String,
    pub role: String,
}

impl RosterMember {
    pub fn new(member_name: impl Into<String>) -> Self {
        Self {
            member_name: member_name.into(),
            display_name: String::new(),
            desc: String::new(),
            role: String::new(),
        }
    }
}

/// 两次名册快照之间的差异。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RosterDelta {
    pub joined: Vec<RosterMember>,
    pub left: Vec<RosterMember>,
    pub changed: Vec<RosterMember>,
}

impl RosterDelta {
    pub fn is_empty(&self) -> bool {
        self.joined.is_empty() && self.left.is_empty() && self.changed.is_empty()
    }
}

/// 名册 diff Seam(Service Definition)。
pub trait RosterDiff: Seam {
    /// 对比新旧名册(键为 member_name;None 视为空)。
    fn diff(&self, old: Option<&[RosterMember]>, new: Option<&[RosterMember]>) -> RosterDelta;

    /// 渲染一行名册条目:`- [prefix] member_name=X display_name=Y [human] :: desc`。
    fn format_member_line(
        &self,
        member: &RosterMember,
        mark_humans: bool,
        prefix: Option<&str>,
    ) -> String;

    /// 全量名册正文(无成员返回 None;标题 + 逐行)。
    fn build_roster_snapshot_text(
        &self,
        members: Option<&[RosterMember]>,
        mark_humans: bool,
        language: &str,
    ) -> Option<String>;

    /// 增量正文(空差异返回 None;joined/left/changed 带前缀)。
    fn build_roster_delta_text(
        &self,
        delta: &RosterDelta,
        mark_humans: bool,
        language: &str,
    ) -> Option<String>;
}
