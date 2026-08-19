//! 进化协议字面量与值集(对齐 agent_evolving/protocols.py)。
//!
//! 纯常量:动作/模式/效果/目标/信号字面量 + 目标/主体类型/简化动作/补丁动作/章节值集校验。

// ---- 动作字面量 ----

pub const APPROVE_ACTION: &str = "approve";
pub const REJECT_ACTION: &str = "reject";
pub const RETRY_ACTION: &str = "retry";

// ---- 更新模式字面量(与 UpdateMode::as_str 一致)----

pub const APPEND_MODE: &str = "append";
pub const MERGE_MODE: &str = "merge";
pub const REPLACE_MODE: &str = "replace";

// ---- 更新效果字面量(与 UpdateEffect::as_str 一致)----

pub const STATE_EFFECT: &str = "state";
pub const PENDING_CHANGE_EFFECT: &str = "pending_change";

// ---- 目标 / 条目字面量 ----

pub const EXPERIENCES_TARGET: &str = "experiences";
pub const EXPERIENCE_ENTRY: &str = "experience_entry";
pub const SKILL_EXPERIENCE_ENTRY: &str = "skill_experience_entry";
pub const LOCAL_APPLY_COMPLETED: &str = "local_apply_completed";

// ---- 信号字面量 ----

pub const CONVERSATION_REVIEW_SIGNAL: &str = "conversation_review";
pub const EXECUTION_FAILURE_SIGNAL: &str = "execution_failure";
pub const TOOL_FAILURE_SIGNAL: &str = "tool_failure";
pub const TRAJECTORY_ISSUE_SIGNAL: &str = "trajectory_issue";
pub const USER_INTENT_SIGNAL: &str = "user_intent";

// ---- 值集 ----

/// 可进化目标(description / body / script)。
pub const EVOLUTION_TARGET_VALUES: [&str; 3] = ["description", "body", "script"];

/// 进化主体类型(skill / team-skill / swarm-skill)。
pub const EVOLUTION_SUBJECT_KIND_VALUES: [&str; 3] = ["skill", "team-skill", "swarm-skill"];

/// 简化动作(DELETE / MERGE / REFINE / KEEP)。
pub const SIMPLIFY_ACTION_VALUES: [&str; 4] = ["DELETE", "MERGE", "REFINE", "KEEP"];

/// 合法补丁动作(append / merge / replace / skip)。
pub const VALID_PATCH_ACTIONS: [&str; 4] = ["append", "merge", "replace", "skip"];

/// 合法技能章节。
pub const VALID_SECTIONS: [&str; 8] = [
    "Instructions",
    "Examples",
    "Troubleshooting",
    "Scripts",
    "Collaboration",
    "Roles",
    "Constraints",
    "Workflow",
];

/// 值是否属于给定值集。
fn in_values(value: &str, values: &[&str]) -> bool {
    values.contains(&value)
}

pub fn is_evolution_target(value: &str) -> bool {
    in_values(value, &EVOLUTION_TARGET_VALUES)
}

pub fn is_evolution_subject_kind(value: &str) -> bool {
    in_values(value, &EVOLUTION_SUBJECT_KIND_VALUES)
}

pub fn is_simplify_action(value: &str) -> bool {
    in_values(value, &SIMPLIFY_ACTION_VALUES)
}

pub fn is_valid_patch_action(value: &str) -> bool {
    in_values(value, &VALID_PATCH_ACTIONS)
}

pub fn is_valid_section(value: &str) -> bool {
    in_values(value, &VALID_SECTIONS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_literals_match_python() {
        assert_eq!(APPROVE_ACTION, "approve");
        assert_eq!(REJECT_ACTION, "reject");
        assert_eq!(RETRY_ACTION, "retry");
        assert_eq!(APPEND_MODE, "append");
        assert_eq!(MERGE_MODE, "merge");
        assert_eq!(REPLACE_MODE, "replace");
        assert_eq!(STATE_EFFECT, "state");
        assert_eq!(PENDING_CHANGE_EFFECT, "pending_change");
        assert_eq!(EXPERIENCES_TARGET, "experiences");
        assert_eq!(EXPERIENCE_ENTRY, "experience_entry");
        assert_eq!(SKILL_EXPERIENCE_ENTRY, "skill_experience_entry");
        assert_eq!(LOCAL_APPLY_COMPLETED, "local_apply_completed");
        assert_eq!(CONVERSATION_REVIEW_SIGNAL, "conversation_review");
        assert_eq!(EXECUTION_FAILURE_SIGNAL, "execution_failure");
        assert_eq!(TOOL_FAILURE_SIGNAL, "tool_failure");
        assert_eq!(TRAJECTORY_ISSUE_SIGNAL, "trajectory_issue");
        assert_eq!(USER_INTENT_SIGNAL, "user_intent");
    }

    #[test]
    fn value_sets_match_python() {
        assert_eq!(EVOLUTION_TARGET_VALUES, ["description", "body", "script"]);
        assert_eq!(
            EVOLUTION_SUBJECT_KIND_VALUES,
            ["skill", "team-skill", "swarm-skill"]
        );
        assert_eq!(
            SIMPLIFY_ACTION_VALUES,
            ["DELETE", "MERGE", "REFINE", "KEEP"]
        );
        assert_eq!(VALID_PATCH_ACTIONS, ["append", "merge", "replace", "skip"]);
        assert_eq!(
            VALID_SECTIONS,
            [
                "Instructions",
                "Examples",
                "Troubleshooting",
                "Scripts",
                "Collaboration",
                "Roles",
                "Constraints",
                "Workflow",
            ]
        );
    }

    #[test]
    fn evolution_target_validation() {
        assert!(is_evolution_target("description"));
        assert!(is_evolution_target("body"));
        assert!(is_evolution_target("script"));
        assert!(!is_evolution_target("experiences"));
        assert!(!is_evolution_target(""));
    }

    #[test]
    fn subject_kind_validation() {
        assert!(is_evolution_subject_kind("skill"));
        assert!(is_evolution_subject_kind("team-skill"));
        assert!(is_evolution_subject_kind("swarm-skill"));
        assert!(!is_evolution_subject_kind("agent"));
    }

    #[test]
    fn simplify_action_validation() {
        assert!(is_simplify_action("DELETE"));
        assert!(is_simplify_action("MERGE"));
        assert!(is_simplify_action("REFINE"));
        assert!(is_simplify_action("KEEP"));
        assert!(!is_simplify_action("delete"));
    }

    #[test]
    fn patch_action_and_section_validation() {
        for a in ["append", "merge", "replace", "skip"] {
            assert!(is_valid_patch_action(a), "{a}");
        }
        assert!(!is_valid_patch_action("delete"));
        for s in ["Instructions", "Examples", "Scripts", "Workflow"] {
            assert!(is_valid_section(s), "{s}");
        }
        assert!(!is_valid_section("Summary"));
    }
}
