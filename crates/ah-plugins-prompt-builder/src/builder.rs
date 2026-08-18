//! 系统提示构建器(1:1 对齐 core/single_agent/prompts/builder.py + harness/prompts/builder.py)。
//!
//! - PromptSection 注册/替换/移除/查询(按名);
//! - build():按 priority 升序排序,join "\n\n",跳过空白 section;
//! - DeepAgent 模式:Full 全部 / Minimal 只保留最小集合 / None 只输出 identity;
//! - build_report():生成诊断报告(委托 report 模块)。

use ah_contracts::prompt_builder::{PromptMode, PromptSection};
use ah_contracts::seam::Seam;

/// DeepAgent 最小模式保留的 section 名集合(对齐 _MINIMAL_SECTIONS)。
pub const MINIMAL_SECTIONS: [&str; 7] = [
    "identity",
    "safety",
    "skills",
    "tools",
    "runtime",
    "prompt_attachments",
    "memory",
];

/// Section 注册 + 排序渲染 + 模式过滤的系统提示构建器。
pub struct SystemPromptBuilder {
    pub(crate) language: String,
    pub(crate) mode: PromptMode,
    pub(crate) sections: std::collections::BTreeMap<String, PromptSection>,
}

impl Seam for SystemPromptBuilder {}

impl SystemPromptBuilder {
    pub fn new(language: impl Into<String>, mode: PromptMode) -> Self {
        Self {
            language: language.into(),
            mode,
            sections: std::collections::BTreeMap::new(),
        }
    }

    /// 添加或替换 section(同名覆盖;对齐 add_section)。
    pub fn add_section(&mut self, section: PromptSection) -> &mut Self {
        self.sections.insert(section.name.clone(), section);
        self
    }

    /// 按名移除 section(不存在则无操作;对齐 remove_section)。
    pub fn remove_section(&mut self, name: &str) -> &mut Self {
        self.sections.remove(name);
        self
    }

    /// 返回全部注册 section 的副本(对齐 get_all_sections)。
    pub fn get_all_sections(&self) -> std::collections::BTreeMap<String, PromptSection> {
        self.sections.clone()
    }

    pub fn has_section(&self, name: &str) -> bool {
        self.sections.contains_key(name)
    }

    pub fn get_section(&self, name: &str) -> Option<&PromptSection> {
        self.sections.get(name)
    }

    /// 当前语言。
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 当前模式。
    pub fn mode(&self) -> PromptMode {
        self.mode
    }

    /// 按当前模式过滤后的 section 列表(对齐 _get_sections_for_build)。
    fn sections_for_build(&self) -> Vec<&PromptSection> {
        match self.mode {
            PromptMode::Full => self.sections.values().collect(),
            PromptMode::Minimal => self
                .sections
                .values()
                .filter(|s| MINIMAL_SECTIONS.contains(&s.name.as_str()))
                .collect(),
            PromptMode::None => self
                .sections
                .values()
                .filter(|s| s.name == "identity")
                .collect(),
        }
    }

    /// 按 priority 升序排序并拼接为完整提示(跳过空白;对齐 build)。
    pub fn build(&self) -> String {
        let mut sections = self.sections_for_build();
        sections.sort_by_key(|s| s.priority);
        let parts: Vec<String> = sections
            .iter()
            .map(|s| s.render(&self.language))
            .filter(|p| !p.trim().is_empty())
            .collect();
        parts.join("\n\n")
    }

    /// 生成诊断报告(对齐 build_report;委托 report 模块)。
    pub fn build_report(&self) -> ah_contracts::prompt_builder::PromptReport {
        crate::report::from_builder(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn section(name: &str, priority: u64, content: &str) -> PromptSection {
        let mut c = BTreeMap::new();
        c.insert("cn".to_string(), content.to_string());
        PromptSection::new(name, c, priority)
    }

    #[test]
    fn add_replace_remove_has_get_flow() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::Full);
        b.add_section(section("identity", 100, "我是身份"));
        assert!(b.has_section("identity"));
        assert_eq!(b.get_section("identity").unwrap().name, "identity");
        b.add_section(section("identity", 100, "新身份"));
        assert_eq!(b.get_section("identity").unwrap().render("cn"), "新身份");
        b.remove_section("identity");
        assert!(!b.has_section("identity"));
        assert!(b.get_section("identity").is_none());
        assert_eq!(b.get_all_sections().len(), 0);
    }

    #[test]
    fn build_sorts_by_priority() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::Full);
        b.add_section(section("tools", 200, "工具"));
        b.add_section(section("identity", 100, "身份"));
        b.add_section(section("memory", 300, "记忆"));
        let out = b.build();
        let i = out.find("身份").unwrap();
        let j = out.find("工具").unwrap();
        let k = out.find("记忆").unwrap();
        assert!(i < j && j < k, "priority 100 < 200 < 300");
    }

    #[test]
    fn build_skips_blank_sections() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::Full);
        b.add_section(section("identity", 100, "身份"));
        b.add_section(section("tools", 200, "   "));
        b.add_section(section("memory", 300, "记忆"));
        let out = b.build();
        assert!(out.contains("身份") && out.contains("记忆"));
        assert_eq!(out.matches("\n\n").count(), 1);
    }

    #[test]
    fn none_mode_only_identity() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::None);
        b.add_section(section("identity", 100, "身份"));
        b.add_section(section("tools", 200, "工具"));
        b.add_section(section("memory", 300, "记忆"));
        let out = b.build();
        assert!(out.contains("身份"));
        assert!(!out.contains("工具") && !out.contains("记忆"));
    }

    #[test]
    fn minimal_mode_only_minimal_set() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::Minimal);
        for (name, prio) in [
            ("identity", 100),
            ("safety", 110),
            ("skills", 120),
            ("tools", 130),
            ("runtime", 140),
            ("prompt_attachments", 150),
            ("memory", 160),
            ("todo", 170),
            ("workspace", 180),
        ] {
            b.add_section(section(name, prio, &format!("{name}-内容")));
        }
        let out = b.build();
        for keep in [
            "identity",
            "safety",
            "skills",
            "tools",
            "runtime",
            "prompt_attachments",
            "memory",
        ] {
            assert!(
                out.contains(&format!("{keep}-内容")),
                "minimal must keep {keep}"
            );
        }
        assert!(!out.contains("todo-内容") && !out.contains("workspace-内容"));
    }

    #[test]
    fn full_mode_all_sections() {
        let mut b = SystemPromptBuilder::new("cn", PromptMode::Full);
        for (name, prio) in [("identity", 100), ("todo", 200), ("workspace", 300)] {
            b.add_section(section(name, prio, &format!("{name}-内容")));
        }
        let out = b.build();
        assert!(
            out.contains("identity-内容")
                && out.contains("todo-内容")
                && out.contains("workspace-内容")
        );
    }

    #[test]
    fn build_report_delegates_with_stats() {
        let mut b = SystemPromptBuilder::new("en", PromptMode::Minimal);
        b.add_section(section("identity", 100, "who"));
        b.add_section(section("tools", 200, "tools here"));
        let report = b.build_report();
        assert_eq!(report.language, "en");
        assert_eq!(report.mode, "minimal");
        assert_eq!(report.section_count, 2);
        assert_eq!(report.total_chars, 3 + 10);
    }
}
