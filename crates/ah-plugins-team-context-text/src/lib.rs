//! # ah-plugins-team-context-text
//!
//! 真实团队上下文消息正文渲染(对齐 openjiuwen/agent_teams/prompts/messages.py 的
//! build_identity_text / build_team_info_text,F_70):
//! - `build_identity_text`:成员自身身份正文;全空 → None;行序 = [identity_heading, "",
//!   可选 member_name 行, 可选 display_name 行, 可选 workspace 行(带父括号用途,
//!   cn 用（）en 用 ()), 可选 [private_prompt_heading, "", prompt] 块];末尾 + "\n";
//! - `build_team_info_text`:团队元数据正文;全空 → None;行序 = [info_heading, "",
//!   可选 team_name 行, 可选 display_name 行, 可选 desc 行, 可选 workspace mount 行
//!   (带用途子行 + 可选绝对路径子行)或仅 path 行];末尾 + "\n";
//! - 双语标签表(cn/en)1:1 摘录自 Python `_LABELS`(identity/info 相关键)。
//!
//! 纯函数、无 IO、无状态,确定性可测。

use std::sync::Arc;

use ah_contracts::keys::TEAM_CONTEXT_TEXT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_context_text::{TeamContextText, TeamInfo};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 双语标签表(对齐 Python messages.py `_LABELS`,仅 identity/info 相关键)。
struct Labels {
    identity_heading: &'static str,
    member_name_line: &'static str,
    display_name_line: &'static str,
    member_workspace_line: &'static str,
    member_workspace_purpose: &'static str,
    private_prompt_heading: &'static str,
    info_heading: &'static str,
    team_name_label: &'static str,
    display_name_label: &'static str,
    team_desc: &'static str,
    team_workspace: &'static str,
    team_workspace_purpose: &'static str,
    team_workspace_abs: &'static str,
}

const LABELS_CN: Labels = Labels {
    identity_heading: "# 成员身份",
    member_name_line: "你的 member_name",
    display_name_line: "你的 display_name",
    member_workspace_line: "你的私有工作区",
    member_workspace_purpose: "存放你自己的产物、记忆与技能视图；团队共享文件走团队共享工作空间，不要放这里，也不要把新 skill 创建到这里",
    private_prompt_heading: "## 私有工作约定",
    info_heading: "# 团队信息",
    team_name_label: "team_name（团队唯一标识）",
    display_name_label: "display_name（团队展示名）",
    team_desc: "团队目标与指令",
    team_workspace: "团队共享工作空间",
    team_workspace_purpose: "用于存放团队共享文件（方案、设计、交付成果），所有成员通过该路径前缀读写同一份文件，系统自动管理版本和文件锁",
    team_workspace_abs: "绝对路径",
};

const LABELS_EN: Labels = Labels {
    identity_heading: "# Member Identity",
    member_name_line: "Your member_name",
    display_name_line: "Your display_name",
    member_workspace_line: "Your private workspace",
    member_workspace_purpose: "Holds your own artifacts, memory and skills view. Team-shared files belong in the team shared workspace, not here, and new skills must not be created here either",
    private_prompt_heading: "## Private Working Agreement",
    info_heading: "# Team Info",
    team_name_label: "team_name (unique identifier)",
    display_name_label: "display_name (human-readable label)",
    team_desc: "Team Goal & Directives",
    team_workspace: "Team Shared Workspace",
    team_workspace_purpose: "Holds team-shared files (plans, designs, deliverables); all members read/write the same files through this path prefix. Versioning and file locks are managed automatically",
    team_workspace_abs: "Absolute path",
};

/// 按语言取标签表;非 "en" 一律回落 "cn"(对齐 Python labels_for)。
fn labels_for(language: &str) -> &'static Labels {
    if language == "en" {
        &LABELS_EN
    } else {
        &LABELS_CN
    }
}

/// 把尾部从句包进该语言实际使用的括号(cn 全角无空格,其余半角带前导空格)。
fn parenthesized(text: &str, language: &str) -> String {
    if language == "cn" {
        format!("（{text}）")
    } else {
        format!(" ({text})")
    }
}

/// 真实渲染器(纯函数)。
pub struct TeamContextTextRenderer;

impl Seam for TeamContextTextRenderer {}

impl TeamContextText for TeamContextTextRenderer {
    fn build_identity_text(
        &self,
        member_name: Option<&str>,
        display_name: Option<&str>,
        member_workspace_path: Option<&str>,
        member_prompt: Option<&str>,
        language: &str,
    ) -> Option<String> {
        // 对齐 Python:member_name 不做 strip(truthiness 即非空串);
        // display_name / workspace / prompt 先 strip,空串视为缺省。
        let name = member_name.unwrap_or("");
        let label = display_name.map_or("", str::trim);
        let workspace = member_workspace_path.map_or("", str::trim);
        let private_prompt = member_prompt.map_or("", str::trim);
        if name.is_empty() && label.is_empty() && workspace.is_empty() && private_prompt.is_empty()
        {
            return None;
        }
        let labels = labels_for(language);
        let mut lines = vec![labels.identity_heading.to_string(), String::new()];
        if !name.is_empty() {
            let line = labels.member_name_line;
            lines.push(format!("{line}: {name}"));
        }
        if !label.is_empty() {
            let line = labels.display_name_line;
            lines.push(format!("{line}: {label}"));
        }
        if !workspace.is_empty() {
            let line = labels.member_workspace_line;
            let purpose = parenthesized(labels.member_workspace_purpose, language);
            lines.push(format!("{line}: `{workspace}`{purpose}"));
        }
        if !private_prompt.is_empty() {
            lines.extend([
                String::new(),
                labels.private_prompt_heading.to_string(),
                String::new(),
                private_prompt.to_string(),
            ]);
        }
        let mut body = lines.join("\n");
        body.push('\n');
        Some(body)
    }

    fn build_team_info_text(
        &self,
        team_info: Option<&TeamInfo>,
        team_workspace_mount: Option<&str>,
        team_workspace_path: Option<&str>,
        language: &str,
    ) -> Option<String> {
        // 对齐 Python:team_name / display_name / desc 用原始值(空串视为缺省);
        // mount / path 先 strip。
        let team_name = team_info
            .and_then(|t| t.team_name.as_deref())
            .filter(|s| !s.is_empty());
        let display_name = team_info
            .and_then(|t| t.display_name.as_deref())
            .filter(|s| !s.is_empty());
        let desc = team_info
            .and_then(|t| t.desc.as_deref())
            .filter(|s| !s.is_empty());
        let mount = team_workspace_mount.map_or("", str::trim);
        let path = team_workspace_path.map_or("", str::trim);
        if team_name.is_none()
            && display_name.is_none()
            && desc.is_none()
            && mount.is_empty()
            && path.is_empty()
        {
            return None;
        }
        let labels = labels_for(language);
        let mut lines = vec![labels.info_heading.to_string(), String::new()];
        if let Some(team_name) = team_name {
            let label = labels.team_name_label;
            lines.push(format!("- {label}: {team_name}"));
        }
        if let Some(display_name) = display_name {
            let label = labels.display_name_label;
            lines.push(format!("- {label}: {display_name}"));
        }
        if let Some(desc) = desc {
            let label = labels.team_desc;
            lines.push(format!("- {label}: {desc}"));
        }
        if !mount.is_empty() {
            let label = labels.team_workspace;
            let purpose = labels.team_workspace_purpose;
            lines.push(format!("- {label}: `{mount}`"));
            lines.push(format!("  - {purpose}"));
            if !path.is_empty() {
                let abs = labels.team_workspace_abs;
                lines.push(format!("  - {abs}: `{path}`"));
            }
        } else if !path.is_empty() {
            let label = labels.team_workspace;
            let purpose = labels.team_workspace_purpose;
            lines.push(format!("- {label}: `{path}`"));
            lines.push(format!("  - {purpose}"));
        }
        let mut body = lines.join("\n");
        body.push('\n');
        Some(body)
    }
}

/// team-context-text 插件:注册 `team-context-text` seam。
pub struct TeamContextTextPlugin;

impl Plugin for TeamContextTextPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-context-text"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_CONTEXT_TEXT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let renderer: Arc<dyn TeamContextText> = Arc::new(TeamContextTextRenderer);
        Ok(vec![ctx.register(TEAM_CONTEXT_TEXT, renderer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_CONTEXT_TEXT;
    use ah_contracts::team_context_text::TeamInfo;
    use ah_hub::plugin::DynPlugin;

    const EN_WORKSPACE_PURPOSE: &str = "Holds team-shared files (plans, designs, deliverables); all members read/write the same files through this path prefix. Versioning and file locks are managed automatically";

    #[test]
    fn identity_all_fields_cn() {
        let renderer = TeamContextTextRenderer;
        let body = renderer
            .build_identity_text(
                Some("alice"),
                Some("Alice"),
                Some("/ws/alice"),
                Some("遵守团队规范，不私自行动。"),
                "cn",
            )
            .expect("identity body");
        let expected = concat!(
            "# 成员身份\n",
            "\n",
            "你的 member_name: alice\n",
            "你的 display_name: Alice\n",
            "你的私有工作区: `/ws/alice`（存放你自己的产物、记忆与技能视图；团队共享文件走团队共享工作空间，不要放这里，也不要把新 skill 创建到这里）\n",
            "\n",
            "## 私有工作约定\n",
            "\n",
            "遵守团队规范，不私自行动。\n",
        );
        assert_eq!(body, expected);
    }

    #[test]
    fn identity_en_labels_and_halfwidth_parens() {
        let renderer = TeamContextTextRenderer;
        let body = renderer
            .build_identity_text(
                Some("alice"),
                Some("Alice"),
                Some("/ws/alice"),
                Some("Follow team rules."),
                "en",
            )
            .expect("identity body");
        assert!(body.starts_with("# Member Identity\n\n"));
        assert!(body.contains("Your member_name: alice\n"));
        assert!(body.contains("Your display_name: Alice\n"));
        assert!(body.contains(
            "Your private workspace: `/ws/alice` (Holds your own artifacts, memory and skills view. Team-shared files belong in the team shared workspace, not here, and new skills must not be created here either)\n"
        ));
        assert!(body.contains("## Private Working Agreement\n\nFollow team rules.\n"));
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn identity_missing_fields_only_name() {
        let renderer = TeamContextTextRenderer;
        let body = renderer
            .build_identity_text(Some("bob"), None, None, None, "cn")
            .expect("identity body");
        assert_eq!(body, "# 成员身份\n\n你的 member_name: bob\n");
    }

    #[test]
    fn identity_member_name_is_not_stripped() {
        let renderer = TeamContextTextRenderer;
        // 对齐 Python:member_name 不做 strip,纯空白仍渲染该行。
        let body = renderer
            .build_identity_text(Some("  "), None, None, None, "cn")
            .expect("identity body");
        assert_eq!(body, "# 成员身份\n\n你的 member_name:   \n");
    }

    #[test]
    fn identity_all_empty_returns_none() {
        let renderer = TeamContextTextRenderer;
        assert!(
            renderer
                .build_identity_text(None, None, None, None, "cn")
                .is_none()
        );
        // 空串 / 纯空白(display / workspace / prompt 先 strip)一律视为缺省。
        assert!(
            renderer
                .build_identity_text(Some(""), Some("  "), Some("  "), Some("   "), "cn")
                .is_none()
        );
    }

    #[test]
    fn identity_workspace_parentheses_cn_vs_en() {
        let renderer = TeamContextTextRenderer;
        let cn = renderer
            .build_identity_text(None, None, Some("/w"), None, "cn")
            .expect("cn body");
        assert!(cn.contains(
            "你的私有工作区: `/w`（存放你自己的产物、记忆与技能视图；团队共享文件走团队共享工作空间，不要放这里，也不要把新 skill 创建到这里）\n"
        ));
        let en = renderer
            .build_identity_text(None, None, Some("/w"), None, "en")
            .expect("en body");
        assert!(en.contains(
            "Your private workspace: `/w` (Holds your own artifacts, memory and skills view. Team-shared files belong in the team shared workspace, not here, and new skills must not be created here either)\n"
        ));
    }

    #[test]
    fn identity_private_prompt_block() {
        let renderer = TeamContextTextRenderer;
        let body = renderer
            .build_identity_text(None, None, None, Some("我的私有约定"), "cn")
            .expect("identity body");
        assert_eq!(body, "# 成员身份\n\n\n## 私有工作约定\n\n我的私有约定\n");
    }

    #[test]
    fn info_all_fields_cn() {
        let renderer = TeamContextTextRenderer;
        let info = TeamInfo {
            team_name: Some("t1".to_string()),
            display_name: Some("T1".to_string()),
            desc: Some("完成调研并交付方案。".to_string()),
        };
        let body = renderer
            .build_team_info_text(Some(&info), Some(".team/t1/"), Some("/abs/team/t1"), "cn")
            .expect("team info body");
        let expected = concat!(
            "# 团队信息\n",
            "\n",
            "- team_name（团队唯一标识）: t1\n",
            "- display_name（团队展示名）: T1\n",
            "- 团队目标与指令: 完成调研并交付方案。\n",
            "- 团队共享工作空间: `.team/t1/`\n",
            "  - ",
            "用于存放团队共享文件（方案、设计、交付成果），所有成员通过该路径前缀读写同一份文件，系统自动管理版本和文件锁",
            "\n",
            "  - 绝对路径: `/abs/team/t1`\n",
        );
        assert_eq!(body, expected);
    }

    #[test]
    fn info_only_team_name() {
        let renderer = TeamContextTextRenderer;
        let info = TeamInfo {
            team_name: Some("t1".to_string()),
            ..Default::default()
        };
        let body = renderer
            .build_team_info_text(Some(&info), None, None, "cn")
            .expect("team info body");
        assert_eq!(body, "# 团队信息\n\n- team_name（团队唯一标识）: t1\n");
    }

    #[test]
    fn info_workspace_mount_with_and_without_path() {
        let renderer = TeamContextTextRenderer;
        let with_path = renderer
            .build_team_info_text(None, Some(".team/t1/"), Some("/abs/t1"), "cn")
            .expect("team info body");
        let expected_with_path = concat!(
            "# 团队信息\n",
            "\n",
            "- 团队共享工作空间: `.team/t1/`\n",
            "  - ",
            "用于存放团队共享文件（方案、设计、交付成果），所有成员通过该路径前缀读写同一份文件，系统自动管理版本和文件锁",
            "\n",
            "  - 绝对路径: `/abs/t1`\n",
        );
        assert_eq!(with_path, expected_with_path);

        let mount_only = renderer
            .build_team_info_text(None, Some(".team/t1/"), None, "cn")
            .expect("team info body");
        let expected_mount_only = concat!(
            "# 团队信息\n",
            "\n",
            "- 团队共享工作空间: `.team/t1/`\n",
            "  - ",
            "用于存放团队共享文件（方案、设计、交付成果），所有成员通过该路径前缀读写同一份文件，系统自动管理版本和文件锁",
            "\n",
        );
        assert_eq!(mount_only, expected_mount_only);
    }

    #[test]
    fn info_path_only_without_mount() {
        let renderer = TeamContextTextRenderer;
        let body = renderer
            .build_team_info_text(None, None, Some("/abs/t1"), "cn")
            .expect("team info body");
        let expected = concat!(
            "# 团队信息\n",
            "\n",
            "- 团队共享工作空间: `/abs/t1`\n",
            "  - ",
            "用于存放团队共享文件（方案、设计、交付成果），所有成员通过该路径前缀读写同一份文件，系统自动管理版本和文件锁",
            "\n",
        );
        assert_eq!(body, expected);
    }

    #[test]
    fn info_all_empty_returns_none() {
        let renderer = TeamContextTextRenderer;
        assert!(
            renderer
                .build_team_info_text(None, None, None, "cn")
                .is_none()
        );
        let empty = TeamInfo::default();
        assert!(
            renderer
                .build_team_info_text(Some(&empty), None, None, "cn")
                .is_none()
        );
        // 空串字段 + 纯空白 mount / path 同样视为缺省。
        let blank = TeamInfo {
            team_name: Some(String::new()),
            ..Default::default()
        };
        assert!(
            renderer
                .build_team_info_text(Some(&blank), Some("  "), Some("  "), "cn")
                .is_none()
        );
    }

    #[test]
    fn info_en_labels() {
        let renderer = TeamContextTextRenderer;
        let info = TeamInfo {
            team_name: Some("t1".to_string()),
            display_name: Some("T1".to_string()),
            desc: Some("Deliver research.".to_string()),
        };
        let body = renderer
            .build_team_info_text(Some(&info), Some(".team/t1/"), Some("/abs/team/t1"), "en")
            .expect("team info body");
        assert!(body.starts_with("# Team Info\n\n"));
        assert!(body.contains("- team_name (unique identifier): t1\n"));
        assert!(body.contains("- display_name (human-readable label): T1\n"));
        assert!(body.contains("- Team Goal & Directives: Deliver research.\n"));
        assert!(body.contains("- Team Shared Workspace: `.team/t1/`\n"));
        assert!(body.contains(EN_WORKSPACE_PURPOSE));
        assert!(body.contains("  - Absolute path: `/abs/team/t1`\n"));
    }

    #[test]
    fn unknown_language_falls_back_to_cn() {
        let renderer = TeamContextTextRenderer;
        let identity = renderer
            .build_identity_text(Some("alice"), None, None, None, "fr")
            .expect("identity body");
        assert!(identity.contains("# 成员身份\n"));
        let info = renderer
            .build_team_info_text(None, Some(".team/"), None, "fr")
            .expect("team info body");
        assert!(info.contains("# 团队信息\n"));
    }

    #[test]
    fn team_info_serde_roundtrip() {
        let json = r#"{"team_name":"t1","display_name":"T1","desc":"d"}"#;
        let info: TeamInfo = serde_json::from_str(json).expect("deserialize");
        assert_eq!(info.team_name.as_deref(), Some("t1"));
        assert_eq!(info.display_name.as_deref(), Some("T1"));
        assert_eq!(info.desc.as_deref(), Some("d"));
        let back = serde_json::to_string(&info).expect("serialize");
        assert!(back.contains("team_name"));
        assert!(back.contains("t1"));
    }

    #[test]
    fn plugin_registers_team_context_text() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamContextTextPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let svc = ctx
            .service::<dyn TeamContextText>(&TEAM_CONTEXT_TEXT)
            .expect("team-context-text seam");
        let body = svc
            .build_identity_text(Some("alice"), None, None, None, "cn")
            .expect("identity body");
        assert_eq!(body, "# 成员身份\n\n你的 member_name: alice\n");
        drop(effects);
        assert!(!ctx.has_service(&TEAM_CONTEXT_TEXT));
    }
}
