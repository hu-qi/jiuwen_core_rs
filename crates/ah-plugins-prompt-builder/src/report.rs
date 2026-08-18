//! 诊断报告(对齐 openjiuwen/harness/prompts/report.py)。
//!
//! 注意:PromptReport / SectionInfo 定义在 ah-contracts(外部 crate),
//! 孤儿规则禁止在本 crate 为其添加 inherent impl,故这里提供自由函数
//! from_builder / summary,语义与 Python 的
//! PromptReport.from_builder(builder) / report.summary() 一一对应。

use ah_contracts::prompt_builder::{PromptMode, PromptReport, SectionInfo};

use crate::builder::SystemPromptBuilder;

/// 粗略估算:1 token ≈ 2.5 个中文字符(对齐 _CN_CHARS_PER_TOKEN)。
const CN_CHARS_PER_TOKEN: f64 = 2.5;
/// 粗略估算:1 token ≈ 4 个英文字符(对齐 _EN_CHARS_PER_TOKEN)。
const EN_CHARS_PER_TOKEN: f64 = 4.0;

/// 从构建器当前状态生成诊断报告(对齐 PromptReport.from_builder)。
///
/// 遍历所有 section 并按 priority 升序统计;language 为 "cn" 时按 2.5
/// 字符/token 估算,否则按 4.0;total_chars 为 0 时 estimated_tokens 为 0。
pub fn from_builder(builder: &SystemPromptBuilder) -> PromptReport {
    let language = builder.language.clone();
    let mode = match builder.mode {
        PromptMode::Full => "full",
        PromptMode::Minimal => "minimal",
        PromptMode::None => "none",
    }
    .to_string();

    let mut section_infos: Vec<SectionInfo> = Vec::new();
    let mut total_chars: usize = 0;
    let mut sections: Vec<&ah_contracts::prompt_builder::PromptSection> =
        builder.sections.values().collect();
    sections.sort_by_key(|s| s.priority);
    for s in sections {
        let chars = s.char_count(&language);
        section_infos.push(SectionInfo {
            name: s.name.clone(),
            priority: s.priority,
            char_count: chars,
        });
        total_chars += chars;
    }

    let chars_per_token = if language == "cn" {
        CN_CHARS_PER_TOKEN
    } else {
        EN_CHARS_PER_TOKEN
    };
    let estimated_tokens = if total_chars == 0 {
        0
    } else {
        (total_chars as f64 / chars_per_token) as usize
    };

    PromptReport {
        total_chars,
        estimated_tokens,
        section_count: section_infos.len(),
        sections: section_infos,
        mode,
        language,
    }
}

/// 人类可读的单行摘要(对齐 PromptReport.summary())。
pub fn summary(report: &PromptReport) -> String {
    format!(
        "[PromptReport] mode={} lang={} sections={} chars={} est_tokens≈{}",
        report.mode,
        report.language,
        report.section_count,
        report.total_chars,
        report.estimated_tokens
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ah_contracts::prompt_builder::{PromptMode, PromptSection};

    use super::{from_builder, summary};
    use crate::builder::SystemPromptBuilder;

    fn section(name: &str, content: &str, language: &str, priority: u64) -> PromptSection {
        PromptSection::new(
            name.to_string(),
            BTreeMap::from([(language.to_string(), content.to_string())]),
            priority,
        )
    }

    fn builder_with(
        language: &str,
        mode: PromptMode,
        sections: Vec<PromptSection>,
    ) -> SystemPromptBuilder {
        let mut map = BTreeMap::new();
        for s in sections {
            map.insert(s.name.clone(), s);
        }
        SystemPromptBuilder {
            language: language.to_string(),
            mode,
            sections: map,
        }
    }

    #[test]
    fn empty_builder_report_is_zeroed() {
        let builder = SystemPromptBuilder::new("cn", PromptMode::Full);
        let report = from_builder(&builder);
        assert_eq!(report.total_chars, 0);
        assert_eq!(report.estimated_tokens, 0);
        assert_eq!(report.section_count, 0);
        assert!(report.sections.is_empty());
        assert_eq!(report.mode, "full");
        assert_eq!(report.language, "cn");
    }

    #[test]
    fn sections_sorted_by_priority() {
        let builder = builder_with(
            "cn",
            PromptMode::Full,
            vec![
                section("tail", "ccc", "cn", 10),
                section("mid", "bbbbb", "cn", 5),
                section("head", "a", "cn", 1),
            ],
        );
        let report = from_builder(&builder);
        let names: Vec<&str> = report.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["head", "mid", "tail"]);
        let priorities: Vec<u64> = report.sections.iter().map(|s| s.priority).collect();
        assert_eq!(priorities, vec![1, 5, 10]);
        let chars: Vec<usize> = report.sections.iter().map(|s| s.char_count).collect();
        assert_eq!(chars, vec![1, 5, 3]);
        assert_eq!(report.total_chars, 9);
        assert_eq!(report.section_count, 3);
    }

    #[test]
    fn cn_language_uses_2_5_chars_per_token() {
        let builder = builder_with(
            "cn",
            PromptMode::Full,
            vec![section("body", "一二三四五", "cn", 1)],
        );
        let report = from_builder(&builder);
        assert_eq!(report.total_chars, 5);
        assert_eq!(report.estimated_tokens, 2);
    }

    #[test]
    fn en_language_uses_4_chars_per_token() {
        let builder = builder_with(
            "en",
            PromptMode::Minimal,
            vec![section("body", "abcdefgh", "en", 1)],
        );
        let report = from_builder(&builder);
        assert_eq!(report.total_chars, 8);
        assert_eq!(report.estimated_tokens, 2);
        assert_eq!(report.mode, "minimal");
        assert_eq!(report.language, "en");
    }

    #[test]
    fn summary_format_matches_python_template() {
        let builder = builder_with(
            "en",
            PromptMode::None,
            vec![section("body", "abcd", "en", 1)],
        );
        let report = from_builder(&builder);
        assert_eq!(
            summary(&report),
            "[PromptReport] mode=none lang=en sections=1 chars=4 est_tokens≈1"
        );
    }

    #[test]
    fn estimated_tokens_truncates_not_rounds() {
        let builder = builder_with(
            "cn",
            PromptMode::Full,
            vec![section("body", "一二三四五六七", "cn", 1)],
        );
        let report = from_builder(&builder);
        assert_eq!(report.total_chars, 7);
        assert_eq!(report.estimated_tokens, 2);
    }
}
