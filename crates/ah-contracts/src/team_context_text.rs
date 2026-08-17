//! team-context-text seam:团队上下文消息正文渲染(对齐 openjiuwen/agent_teams/prompts/messages.py 的
//! build_identity_text / build_team_info_text,F_70)。
//!
//! 两个纯渲染函数(无 IO、无状态):
//! - `build_identity_text`:成员自身身份正文(两个名字 + 私有工作区 + 私有工作约定);
//! - `build_team_info_text`:团队元数据正文(团队名 / 展示名 / 目标指令 + 共享工作空间)。
//!
//! 双语标签表(cn/en)与括号方言(全角「（）」/半角「 ()」)属于文案与渲染算法,
//! 由插件提供(如 ah-plugins-team-context-text),契约层零实现。

use crate::seam::Seam;

/// 本 seam 的服务键(定义于 crate::keys,此处再导出供契约路径使用)。
pub use crate::keys::TEAM_CONTEXT_TEXT;

/// 团队元数据(对齐 Python messages.py 中 `team_info` dict 的 shape)。
///
/// 三个字段均为可选:为空(或空串)时对应行不渲染。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TeamInfo {
    /// 团队唯一标识。
    pub team_name: Option<String>,
    /// 团队展示名。
    pub display_name: Option<String>,
    /// 团队目标与指令。
    pub desc: Option<String>,
}

impl TeamInfo {
    /// 是否没有任何可用字段(全部为 None 或空串)。
    pub fn is_empty(&self) -> bool {
        self.team_name.as_deref().is_none_or(str::is_empty)
            && self.display_name.as_deref().is_none_or(str::is_empty)
            && self.desc.as_deref().is_none_or(str::is_empty)
    }
}

/// team-context-text Seam(Service Definition):纯函数正文渲染。
pub trait TeamContextText: Seam {
    /// 渲染成员自身身份正文;无任何字段时返回 None。
    ///
    /// 行序 = [identity_heading, "", 可选 member_name 行, 可选 display_name 行,
    /// 可选 workspace 行(带父括号用途,cn 用（）en 用 ()), 可选 [private_prompt_heading,
    /// "", prompt] 块];末尾追加 "\n"。
    fn build_identity_text(
        &self,
        member_name: Option<&str>,
        display_name: Option<&str>,
        member_workspace_path: Option<&str>,
        member_prompt: Option<&str>,
        language: &str,
    ) -> Option<String>;

    /// 渲染团队元数据正文;无任何可用字段时返回 None。
    ///
    /// 行序 = [info_heading, "", 可选 team_name 行, 可选 display_name 行, 可选 desc 行,
    /// 可选 workspace mount 行(带用途子行 + 可选绝对路径子行)或仅 path 行];末尾追加 "\n"。
    fn build_team_info_text(
        &self,
        team_info: Option<&TeamInfo>,
        team_workspace_mount: Option<&str>,
        team_workspace_path: Option<&str>,
        language: &str,
    ) -> Option<String>;
}
