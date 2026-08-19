//! 进化信号类型与检测(对齐 agent_evolving/signal/base.py + from_eval.py)。
//!
//! 纯逻辑:EvolutionSignal(类型/章节/摘录/技能名/上下文)+ 指纹去重 + 离线 EvaluatedCase 适配。

use crate::dataset::EvaluatedCase;
use serde_json::Value;
use std::collections::BTreeMap;

/// 向后兼容类别枚举(对齐 EvolutionCategory)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionCategory {
    SkillExperience,
    NewSkill,
}

impl EvolutionCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SkillExperience => "skill_experience",
            Self::NewSkill => "new_skill",
        }
    }
}

/// 经验目标层(对齐 EvolutionTarget)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionTarget {
    Description,
    Body,
    Script,
}

impl EvolutionTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Body => "body",
            Self::Script => "script",
        }
    }
}

/// 检测到的进化信号(对齐 EvolutionSignal)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvolutionSignal {
    pub signal_type: String,
    pub section: String,
    pub excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<BTreeMap<String, Value>>,
}

impl EvolutionSignal {
    /// 序列化字典(context 为 None 时省略;对齐 to_dict)。
    pub fn to_dict(&self) -> BTreeMap<String, Value> {
        let mut d = BTreeMap::new();
        d.insert("type".to_string(), Value::String(self.signal_type.clone()));
        d.insert("section".to_string(), Value::String(self.section.clone()));
        d.insert("excerpt".to_string(), Value::String(self.excerpt.clone()));
        d.insert(
            "skill_name".to_string(),
            self.skill_name
                .as_ref()
                .map(|s| Value::String(s.clone()))
                .unwrap_or(Value::Null),
        );
        if let Some(ctx) = &self.context {
            d.insert(
                "context".to_string(),
                Value::Object(ctx.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            );
        }
        d
    }
}

/// 创建带归一化来源元数据的信号(对齐 make_evolution_signal)。
pub fn make_evolution_signal(
    signal_type: impl Into<String>,
    section: impl Into<String>,
    excerpt: impl Into<String>,
    tool_name: Option<&str>,
    skill_name: Option<&str>,
    source: Option<&str>,
    context: Option<BTreeMap<String, Value>>,
) -> EvolutionSignal {
    let mut merged = context.unwrap_or_default();
    if let Some(s) = source {
        merged
            .entry("source".to_string())
            .or_insert_with(|| Value::String(s.to_string()));
    }
    if let Some(t) = tool_name {
        merged
            .entry("tool_name".to_string())
            .or_insert_with(|| Value::String(t.to_string()));
    }
    EvolutionSignal {
        signal_type: signal_type.into(),
        section: section.into(),
        excerpt: excerpt.into(),
        skill_name: skill_name.map(|s| s.to_string()),
        context: if merged.is_empty() {
            None
        } else {
            Some(merged)
        },
    }
}

/// 读取来源元数据(带向后兼容回退;对齐 get_signal_source)。
pub fn get_signal_source(signal: &EvolutionSignal) -> Option<String> {
    let ctx = signal.context.as_ref()?;
    let source = ctx.get("source")?;
    match source {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// 信号去重指纹(signal_type, tool_name, skill_name, excerpt 前 200 字符;对齐 make_signal_fingerprint)。
pub fn make_signal_fingerprint(signal: &EvolutionSignal) -> (String, String, String, String) {
    let context = signal.context.as_ref();
    let tool_name = context
        .and_then(|c| c.get("tool_name"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let excerpt: String = signal.excerpt.chars().take(200).collect();
    (
        signal.signal_type.clone(),
        tool_name,
        signal.skill_name.clone().unwrap_or_default(),
        excerpt,
    )
}

/// 离线 EvaluatedCase → EvolutionSignal(score >= threshold 时过滤;对齐 from_evaluated_case)。
pub fn from_evaluated_case(
    case: &EvaluatedCase,
    operator_id: &str,
    score_threshold: Option<f64>,
) -> Option<EvolutionSignal> {
    if let Some(t) = score_threshold
        && case.score >= t
    {
        return None;
    }
    let signal_type = if case.score == 0.0 {
        "low_score"
    } else {
        "evaluated"
    };
    let inputs_str = serde_json::to_string(case.inputs()).unwrap_or_default();
    let label_str = serde_json::to_string(case.label()).unwrap_or_default();
    let answer_str = case
        .answer
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let mut context = BTreeMap::new();
    context.insert("question".to_string(), Value::String(inputs_str));
    context.insert("label".to_string(), Value::String(label_str));
    context.insert("answer".to_string(), Value::String(answer_str));
    context.insert("reason".to_string(), Value::String(case.reason.clone()));
    context.insert("score".to_string(), Value::from(case.score));
    Some(make_evolution_signal(
        signal_type,
        "Troubleshooting",
        format!("score={:.2}", case.score),
        None,
        if operator_id.is_empty() {
            None
        } else {
            Some(operator_id)
        },
        Some("offline_evaluation"),
        Some(context),
    ))
}

/// 批量转换(对齐 from_evaluated_cases)。
pub fn from_evaluated_cases(
    cases: &[EvaluatedCase],
    operator_id: &str,
    score_threshold: Option<f64>,
) -> Vec<EvolutionSignal> {
    cases
        .iter()
        .filter_map(|c| from_evaluated_case(c, operator_id, score_threshold))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Case;
    use serde_json::json;

    fn eval_case(score: f64) -> EvaluatedCase {
        let c = Case::new(
            BTreeMap::from([("q".to_string(), json!("question"))]),
            BTreeMap::from([("a".to_string(), json!("answer"))]),
            None,
        );
        EvaluatedCase::new(c, Some(json!("model-out")), score, "reason", None)
    }

    #[test]
    fn make_signal_merges_source_and_tool() {
        let s = make_evolution_signal(
            "user_intent",
            "Instructions",
            "excerpt",
            Some("t1"),
            Some("sk"),
            Some("conv"),
            None,
        );
        assert_eq!(s.signal_type, "user_intent");
        let ctx = s.context.as_ref().unwrap();
        assert_eq!(ctx["source"], "conv");
        assert_eq!(ctx["tool_name"], "t1");
        assert_eq!(s.skill_name.as_deref(), Some("sk"));
    }

    #[test]
    fn context_none_when_empty() {
        let s = make_evolution_signal("x", "y", "z", None, None, None, None);
        assert!(s.context.is_none());
        let d = s.to_dict();
        assert_eq!(d["type"], "x");
        assert!(!d.contains_key("context"));
    }

    #[test]
    fn signal_source_reads_metadata() {
        let s = make_evolution_signal("a", "b", "c", None, None, Some("offline"), None);
        assert_eq!(get_signal_source(&s).as_deref(), Some("offline"));
        let empty = make_evolution_signal("a", "b", "c", None, None, None, None);
        assert_eq!(get_signal_source(&empty), None);
    }

    #[test]
    fn fingerprint_truncates_excerpt() {
        let long = "x".repeat(300);
        let s = make_evolution_signal(
            "execution_failure",
            "Troubleshooting",
            long,
            Some("toolA"),
            Some("sk"),
            None,
            None,
        );
        let (t, tool, skill, excerpt) = make_signal_fingerprint(&s);
        assert_eq!(t, "execution_failure");
        assert_eq!(tool, "toolA");
        assert_eq!(skill, "sk");
        assert_eq!(excerpt.chars().count(), 200);
    }

    #[test]
    fn from_eval_low_score_and_threshold() {
        let low = eval_case(0.0);
        let s = from_evaluated_case(&low, "op1", None).unwrap();
        assert_eq!(s.signal_type, "low_score");
        assert_eq!(s.section, "Troubleshooting");
        assert_eq!(s.excerpt, "score=0.00");
        assert_eq!(s.skill_name.as_deref(), Some("op1"));
        assert_eq!(get_signal_source(&s).as_deref(), Some("offline_evaluation"));
        let ctx = s.context.as_ref().unwrap();
        assert_eq!(ctx["score"], json!(0.0));
    }

    #[test]
    fn from_eval_evaluated_signal_and_threshold_filter() {
        let mid = eval_case(0.5);
        let s = from_evaluated_case(&mid, "", None).unwrap();
        assert_eq!(s.signal_type, "evaluated");
        assert!(s.skill_name.is_none());
        assert_eq!(s.excerpt, "score=0.50");
        // threshold 过滤:score >= threshold 不产生信号
        assert!(from_evaluated_case(&mid, "", Some(0.5)).is_none());
        assert!(from_evaluated_case(&mid, "", Some(0.6)).is_some());
    }

    #[test]
    fn batch_from_evaluated_cases() {
        let cases = vec![eval_case(0.0), eval_case(0.8), eval_case(0.2)];
        let all = from_evaluated_cases(&cases, "op", None);
        assert_eq!(all.len(), 3);
        let filtered = from_evaluated_cases(&cases, "op", Some(0.5));
        assert_eq!(filtered.len(), 2);
    }
}
