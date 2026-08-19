//! 在线演进编排纯逻辑(对齐 experience/online_orchestrator.py 的确定性部分)。
//!
//! 纯逻辑:preferred 信号选择(user_intent 优先,否则首个)+ 信号类型/来源派生 +
//! evolve 状态决策守卫;store/manager/updater 留待集成。

use crate::protocols::USER_INTENT_SIGNAL;
use crate::signal::{EvolutionSignal, get_signal_source};

/// 优先信号(对齐 _get_preferred_signal:首个 user_intent 信号,否则首个信号,空 → None)。
pub fn get_preferred_signal(signals: &[EvolutionSignal]) -> Option<&EvolutionSignal> {
    signals
        .iter()
        .find(|s| s.signal_type == USER_INTENT_SIGNAL)
        .or_else(|| signals.first())
}

/// 信号类型(对齐 _get_signal_type)。
pub fn get_signal_type(signals: &[EvolutionSignal]) -> Option<String> {
    get_preferred_signal(signals).map(|s| s.signal_type.clone())
}

/// 信号来源(对齐 _get_signal_source)。
pub fn get_signal_source_of(signals: &[EvolutionSignal]) -> Option<String> {
    get_preferred_signal(signals).and_then(get_signal_source)
}

/// evolve 状态决策守卫(对齐 evolve() 的开头守卫;返回跳过原因消息或 None 继续)。
pub fn evolve_skip_guard(
    skill_name: &str,
    signals_empty: bool,
    skill_exists: bool,
    skill_definition_exists: bool,
) -> Option<(String, String)> {
    // 返回 (status, message)
    if skill_name.is_empty() || signals_empty {
        return Some((
            "skipped_no_input".to_string(),
            "online evolution skipped because skill_name or signals are empty".to_string(),
        ));
    }
    if !skill_exists {
        return Some((
            "skipped_skill_not_found".to_string(),
            format!("online evolution skipped because skill '{skill_name}' does not exist"),
        ));
    }
    if !skill_definition_exists {
        return Some((
            "skipped_skill_definition_not_found".to_string(),
            format!("online evolution skipped because skill '{skill_name}' is missing SKILL.md"),
        ));
    }
    None
}

/// 结果阶段决策(对齐 evolve() 的后置分支)。
#[derive(Debug, Clone, PartialEq)]
pub enum EvolveOutcome {
    /// 生成失败(generation_failed)。
    GenerationFailed(String),
    /// 无演进记录(no_evolution_no_records)。
    NoEvolution,
    /// 需要审批:staged。
    Staged,
    /// 自动批准成功:auto_approved。
    AutoApproved,
    /// 持久化失败:persistence_failed。
    PersistenceFailed(String),
}

/// 根据生成预览与审批结果推导最终阶段(对齐 evolve() 后置逻辑)。
pub fn decide_evolve_outcome(
    preview_records_empty: bool,
    requires_approval: bool,
    apply_ok: bool,
    apply_errors: &[String],
    generation_error: Option<&str>,
) -> EvolveOutcome {
    if let Some(err) = generation_error {
        return EvolveOutcome::GenerationFailed(err.to_string());
    }
    if preview_records_empty {
        return EvolveOutcome::NoEvolution;
    }
    if requires_approval {
        return EvolveOutcome::Staged;
    }
    if !apply_ok {
        let message = if apply_errors.is_empty() {
            "persistence failed".to_string()
        } else {
            apply_errors.join("; ")
        };
        return EvolveOutcome::PersistenceFailed(message);
    }
    EvolveOutcome::AutoApproved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::make_evolution_signal;

    fn sig(t: &str, source: &str) -> EvolutionSignal {
        make_evolution_signal(t, "S", "excerpt", None, None, Some(source), None)
    }

    #[test]
    fn preferred_signal_prefers_user_intent() {
        let signals = vec![
            sig("execution_failure", "a"),
            sig("user_intent", "b"),
            sig("low_score", "c"),
        ];
        let preferred = get_preferred_signal(&signals).unwrap();
        assert_eq!(preferred.signal_type, "user_intent");
        assert_eq!(get_signal_type(&signals).as_deref(), Some("user_intent"));
        assert_eq!(get_signal_source_of(&signals).as_deref(), Some("b"));
    }

    #[test]
    fn preferred_signal_falls_back_to_first() {
        let signals = vec![sig("low_score", "x"), sig("evaluated", "y")];
        assert_eq!(get_signal_type(&signals).as_deref(), Some("low_score"));
        assert_eq!(get_signal_source_of(&signals).as_deref(), Some("x"));
    }

    #[test]
    fn preferred_signal_empty() {
        assert_eq!(get_preferred_signal(&[]), None);
        assert_eq!(get_signal_type(&[]), None);
        assert_eq!(get_signal_source_of(&[]), None);
    }

    #[test]
    fn skip_guards() {
        let no_input = evolve_skip_guard("", false, true, true).unwrap();
        assert_eq!(no_input.0, "skipped_no_input");
        let no_signals = evolve_skip_guard("sk", true, true, true).unwrap();
        assert_eq!(no_signals.0, "skipped_no_input");
        let no_skill = evolve_skip_guard("sk", false, false, true).unwrap();
        assert_eq!(no_skill.0, "skipped_skill_not_found");
        let no_def = evolve_skip_guard("sk", false, true, false).unwrap();
        assert_eq!(no_def.0, "skipped_skill_definition_not_found");
        assert_eq!(evolve_skip_guard("sk", false, true, true), None);
    }

    #[test]
    fn outcome_decisions() {
        assert_eq!(
            decide_evolve_outcome(false, true, false, &[], Some("boom")),
            EvolveOutcome::GenerationFailed("boom".to_string()),
        );
        assert_eq!(
            decide_evolve_outcome(true, false, false, &[], None),
            EvolveOutcome::NoEvolution,
        );
        assert_eq!(
            decide_evolve_outcome(false, true, false, &[], None),
            EvolveOutcome::Staged,
        );
        assert_eq!(
            decide_evolve_outcome(false, false, true, &[], None),
            EvolveOutcome::AutoApproved,
        );
        assert_eq!(
            decide_evolve_outcome(
                false,
                false,
                false,
                &["e1".to_string(), "e2".to_string()],
                None
            ),
            EvolveOutcome::PersistenceFailed("e1; e2".to_string()),
        );
        assert_eq!(
            decide_evolve_outcome(false, false, false, &[], None),
            EvolveOutcome::PersistenceFailed("persistence failed".to_string()),
        );
    }
}
