//! 系统提示 section 构建器(对齐 harness/prompts/sections/*.py)。
//!
//! 每个 section 提供双语内容常量 + build_*_section(language) -> PromptSection。
//! 素材来自 Python 源码,priority 与 SectionName 逐一对齐。

pub mod advanced;
pub mod base;
pub mod runtime;
pub mod workspace;

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, section: &ah_contracts::prompt_builder::PromptSection) {
        assert_eq!(section.name, name, "section name mismatch for {name}");
        assert!(
            section.content.contains_key("cn"),
            "{name} missing cn content"
        );
        assert!(
            section.content.contains_key("en"),
            "{name} missing en content"
        );
        assert!(
            !section.render("cn").trim().is_empty(),
            "{name} cn content empty"
        );
        assert!(
            !section.render("en").trim().is_empty(),
            "{name} en content empty"
        );
    }

    #[test]
    fn base_sections_build_valid() {
        check("identity", &base::build_identity_section());
        check("safety", &base::build_safety_section());
        check("skills", &base::build_skills_section());
        check("todo", &base::build_todo_section());
        check("task_tool", &base::build_task_tool_section());
        check("session_tools", &base::build_session_tools_section());
    }

    #[test]
    fn runtime_sections_build_valid() {
        check("heartbeat", &runtime::build_heartbeat_section());
        check("memory", &runtime::build_memory_section());
        check("memory", &runtime::build_coding_memory_section());
        check(
            "prompt_attachments",
            &runtime::build_prompt_attachments_section(),
        );
        check("context", &runtime::build_offload_section());
        check("context", &runtime::build_reload_section());
        check("context", &runtime::build_compression_recall_section());
    }

    #[test]
    fn advanced_sections_build_valid() {
        check("mode_instructions", &advanced::build_agent_mode_section());
        check("goal_protocol", &advanced::build_goal_section());
        // external_memory 是参数化构建(见下方专项测试)
        check(
            "completion_signal",
            &advanced::build_task_completion_section(),
        );
        check(
            "progressive_tool_rules",
            &advanced::build_progressive_tool_rules_section(),
        );
    }

    #[test]
    fn external_memory_parameterized() {
        assert!(advanced::build_external_memory_section("", "cn").is_none());
        assert!(advanced::build_external_memory_section("   ", "en").is_none());
        let s = advanced::build_external_memory_section("remember X", "cn").unwrap();
        assert_eq!(s.name, "external_memory");
        assert_eq!(s.priority, 55);
        assert_eq!(s.render("cn"), "remember X");
    }

    #[test]
    fn identity_priority_is_10() {
        assert_eq!(base::build_identity_section().priority, 10);
    }

    #[test]
    fn safety_content_differs_by_language() {
        let s = base::build_safety_section();
        assert_ne!(s.render("cn"), s.render("en"));
    }

    #[test]
    fn todo_mentions_todo_create() {
        let s = base::build_todo_section();
        assert!(s.render("cn").contains("todo_create"));
        assert!(s.render("en").contains("todo_create"));
    }

    #[test]
    fn heartbeat_mentions_heartbeat_ok() {
        let s = runtime::build_heartbeat_section();
        assert!(s.render("cn").contains("HEARTBEAT_OK"));
        assert!(s.render("en").contains("HEARTBEAT_OK"));
    }
}
