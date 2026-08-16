//! # ah-plugins-signals
//!
//! Real evolution-signal mapping (aligned with openjiuwen/rsi/team_skill_optimizer/signals.py):
//! maps analyzer issues to trajectory_issue evolution signals:
//! - issue_type: attribution.target_ref + category keyword attribution;
//! - normalize_trajectory_issue: severity normalization + type/description/affected_role;
//! - excerpts and user-query building (consumed by the LLM evolution step).

use std::sync::Arc;

use ah_contracts::keys::SIGNALS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::signals::{EvolutionSignal, Signals};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Map, Value, json};

/// Normalized issue constant keys.
pub const ISSUE_TYPE_KEY: &str = "issue_type";
pub const ISSUE_DESCRIPTION_KEY: &str = "description";
pub const AFFECTED_ROLE_KEY: &str = "affected_role";
pub const SEVERITY_KEY: &str = "severity";

/// Signal source (aligned with ANALYSIS_SIGNAL_SOURCE).
pub const ANALYSIS_SIGNAL_SOURCE: &str = "analysis";

fn str_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Real evolution-signal implementation.
pub struct TeamSkillSignals;

impl Seam for TeamSkillSignals {}

impl Signals for TeamSkillSignals {
    fn issue_type(&self, issue: &Value) -> String {
        let attribution = issue.get("metadata").and_then(|m| m.get("attribution"));
        let target_ref = attribution
            .and_then(|a| a.get("target_ref"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let category = str_of(issue, "category");
        let lookup = format!("{} {}", target_ref, category).to_lowercase();
        if lookup.contains("routing_policy") {
            return "routing_policy".to_string();
        }
        if lookup.contains("handoff_protocol") {
            return "handoff_protocol".to_string();
        }
        if lookup.contains("shared_context_contract") {
            return "shared_context_contract".to_string();
        }
        if lookup.contains("final_answer_verification") {
            return "final_answer_verification".to_string();
        }
        if lookup.contains("stop_condition") {
            return "stop_condition".to_string();
        }
        "team_coordination".to_string()
    }

    fn normalize_trajectory_issue(&self, issue: &Value) -> Value {
        let severity = str_of(issue, "severity").to_lowercase();
        let severity = if matches!(severity.as_str(), "low" | "medium" | "high") {
            severity
        } else {
            "medium".to_string()
        };
        json!({
            ISSUE_TYPE_KEY: self.issue_type(issue),
            ISSUE_DESCRIPTION_KEY: self.issue_description(issue),
            AFFECTED_ROLE_KEY: self.affected_role(issue),
            SEVERITY_KEY: severity,
        })
    }

    fn issue_description(&self, issue: &Value) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (label, key) in [("summary", "summary"), ("recommendation", "recommendation")] {
            let value = str_of(issue, key);
            if !value.is_empty() {
                parts.push(format!("{label}: {value}"));
            }
        }
        let attribution = issue.get("metadata").and_then(|m| m.get("attribution"));
        if let Some(attribution) = attribution {
            for key in [
                "general_mechanism",
                "root_cause",
                "critical_mistake",
                "target_ref",
            ] {
                let value = str_of(attribution, key);
                if !value.is_empty() {
                    parts.push(format!("{key}: {value}"));
                }
            }
        }
        if parts.is_empty() {
            self.issue_excerpt(issue)
        } else {
            parts.join("\n")
        }
    }

    fn affected_role(&self, issue: &Value) -> String {
        if let Some(first) = issue
            .get("affected_components")
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(Value::as_str)
        {
            return first.to_string();
        }
        if let Some(evidence) = issue.get("evidence") {
            if let Some(component) = evidence.get("affected_component").and_then(Value::as_str) {
                return component.to_string();
            }
            if let Some(list) = evidence.as_array() {
                for item in list {
                    if let Some(component) = item.get("affected_component").and_then(Value::as_str)
                    {
                        return component.to_string();
                    }
                }
            }
        }
        String::new()
    }

    fn issue_excerpt(&self, issue: &Value) -> String {
        for key in ["summary", "recommendation", "description"] {
            let value = str_of(issue, key);
            if !value.is_empty() {
                return value.chars().take(1000).collect();
            }
        }
        let id = str_of(issue, "issue_id");
        let id = if id.is_empty() {
            str_of(issue, "id")
        } else {
            id
        };
        let id = if id.is_empty() {
            "team_skill_issue".to_string()
        } else {
            id
        };
        format!("Analyzer Team Skill issue: {id}")
    }

    fn issue_ids(&self, issues: &[Value]) -> Vec<String> {
        issues
            .iter()
            .enumerate()
            .map(|(index, issue)| {
                let id = str_of(issue, "issue_id");
                let id = if id.is_empty() {
                    str_of(issue, "id")
                } else {
                    id
                };
                if id.is_empty() {
                    format!("team_skill_issue_{:03}", index + 1)
                } else {
                    id
                }
            })
            .collect()
    }

    fn build_user_query(&self, issues: &[Value]) -> String {
        let recommendations: Vec<String> = issues
            .iter()
            .map(|issue| str_of(issue, "recommendation"))
            .filter(|r| !r.is_empty())
            .collect();
        if !recommendations.is_empty() {
            recommendations
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            issues
                .iter()
                .map(|issue| format!("- {}", self.issue_excerpt(issue)))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    fn build_signals(
        &self,
        issues: &[Value],
        skill_name: &str,
        skill_content: &str,
        eval_ref_path: &str,
        analysis_result_path: &str,
    ) -> Vec<EvolutionSignal> {
        issues
            .iter()
            .map(|issue| {
                let mut context: Map<String, Value> = Map::new();
                context.insert("source".to_string(), json!(ANALYSIS_SIGNAL_SOURCE));
                context.insert(
                    "trajectory_issues".to_string(),
                    json!([self.normalize_trajectory_issue(issue)]),
                );
                context.insert("skill_content".to_string(), json!(skill_content));
                context.insert("analysis_issue".to_string(), issue.clone());
                context.insert("eval_ref_path".to_string(), json!(eval_ref_path));
                context.insert(
                    "analysis_result_path".to_string(),
                    json!(analysis_result_path),
                );
                EvolutionSignal {
                    signal_type: "trajectory_issue".to_string(),
                    section: String::new(),
                    excerpt: self.issue_excerpt(issue),
                    skill_name: Some(skill_name.to_string()),
                    context: Some(context),
                }
            })
            .collect()
    }
}

/// signals plugin: registers the `signals` seam.
pub struct SignalsPlugin;

impl Plugin for SignalsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-signals"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SIGNALS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let signals: Arc<dyn Signals> = Arc::new(TeamSkillSignals);
        Ok(vec![ctx.register(SIGNALS, signals)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SIGNALS;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn issue_with(extra: serde_json::Value) -> Value {
        let mut base = json!({
            "issue_id": "i-1",
            "summary": "team coordination failed",
            "recommendation": "route to planner",
            "category": "team_skill.routing_policy",
            "severity": "high",
        });
        if let (Value::Object(map), Value::Object(extra_map)) = (&mut base, &extra) {
            for (k, v) in extra_map {
                map.insert(k.clone(), v.clone());
            }
        }
        base
    }

    #[test]
    fn issue_type_attribution_by_keyword() {
        let signals = TeamSkillSignals;
        assert_eq!(signals.issue_type(&issue_with(json!({}))), "routing_policy");
        assert_eq!(
            signals.issue_type(&issue_with(
                json!({"category": "team_skill.handoff_protocol"})
            )),
            "handoff_protocol"
        );
        assert_eq!(
            signals.issue_type(&issue_with(json!({"category": "x", "metadata": {"attribution": {"target_ref": "team_skill.stop_condition"}}}))),
            "stop_condition"
        );
        // 无匹配 → team_coordination。
        assert_eq!(
            signals.issue_type(&issue_with(json!({"category": "general"}))),
            "team_coordination"
        );
    }

    #[test]
    fn normalize_trajectory_issue_normalizes_severity() {
        let signals = TeamSkillSignals;
        let normalized = signals.normalize_trajectory_issue(&issue_with(json!({})));
        assert_eq!(normalized["severity"], "high");
        assert_eq!(normalized["issue_type"], "routing_policy");
        assert!(
            normalized["description"]
                .as_str()
                .unwrap()
                .contains("summary: ")
        );
        // 非法 severity → medium。
        let bad = signals.normalize_trajectory_issue(&issue_with(json!({"severity": "urgent"})));
        assert_eq!(bad["severity"], "medium");
    }

    #[test]
    fn issue_description_joins_fields_and_falls_back() {
        let signals = TeamSkillSignals;
        let with_attr = issue_with(json!({
            "metadata": { "attribution": { "root_cause": "missing routing", "target_ref": "team_skill.routing_policy" } },
        }));
        let desc = signals.issue_description(&with_attr);
        assert!(desc.contains("summary: team coordination failed"));
        assert!(desc.contains("recommendation: route to planner"));
        assert!(
            desc.contains("root_cause: missing routing"),
            "attribution 字段拼接: {desc}"
        );
        // 无任何字段 → 回退摘录。
        let empty = json!({ "issue_id": "x" });
        let fallback = signals.issue_description(&empty);
        assert!(fallback.contains("Analyzer Team Skill issue: x"));
    }

    #[test]
    fn affected_role_from_components_and_evidence() {
        let signals = TeamSkillSignals;
        let with_components = issue_with(json!({"affected_components": ["planner"]}));
        assert_eq!(signals.affected_role(&with_components), "planner");
        let with_evidence = issue_with(json!({"evidence": {"affected_component": "executor"}}));
        assert_eq!(signals.affected_role(&with_evidence), "executor");
        let with_evidence_list =
            issue_with(json!({"evidence": [{"affected_component": "monitor"}]}));
        assert_eq!(signals.affected_role(&with_evidence_list), "monitor");
        assert_eq!(signals.affected_role(&json!({})), "");
    }

    #[test]
    fn issue_excerpt_priority_and_truncation() {
        let signals = TeamSkillSignals;
        let with_desc = json!({ "description": "desc only" });
        assert_eq!(signals.issue_excerpt(&with_desc), "desc only");
        // summary 优先。
        let both = issue_with(json!({}));
        assert_eq!(signals.issue_excerpt(&both), "team coordination failed");
        // 截断 1000 字符。
        let long = json!({ "summary": "x".repeat(1200) });
        assert_eq!(signals.issue_excerpt(&long).chars().count(), 1000);
        // 回退 id。
        assert_eq!(
            signals.issue_excerpt(&json!({})),
            "Analyzer Team Skill issue: team_skill_issue"
        );
    }

    #[test]
    fn issue_ids_with_fallback() {
        let signals = TeamSkillSignals;
        let issues = vec![
            issue_with(json!({})),
            json!({ "id": "explicit" }),
            json!({}),
        ];
        let ids = signals.issue_ids(&issues);
        assert_eq!(ids, vec!["i-1", "explicit", "team_skill_issue_003"]);
    }

    #[test]
    fn build_user_query_prefers_recommendations() {
        let signals = TeamSkillSignals;
        let issues = vec![issue_with(json!({})), json!({ "summary": "no rec" })];
        let query = signals.build_user_query(&issues);
        assert!(
            query.contains("- route to planner"),
            "recommendation 前缀: {query}"
        );
        assert!(!query.contains("no rec"), "有 recommendation 时不用摘录");
        // 无 recommendation → 摘录。
        let query2 = signals.build_user_query(&[json!({ "summary": "only summary" })]);
        assert!(query2.contains("- only summary"));
    }

    #[test]
    fn build_signals_creates_trajectory_issue_signals() {
        let signals = TeamSkillSignals;
        let issues = vec![
            issue_with(json!({})),
            issue_with(json!({"issue_id": "i-2"})),
        ];
        let built = signals.build_signals(
            &issues,
            "swarm-skill",
            "content",
            "eval.json",
            "analysis.json",
        );
        assert_eq!(built.len(), 2);
        let first = &built[0];
        assert_eq!(first.signal_type, "trajectory_issue");
        assert_eq!(first.skill_name.as_deref(), Some("swarm-skill"));
        let ctx = first.context.as_ref().expect("context");
        assert_eq!(ctx["source"], "analysis");
        assert_eq!(ctx["eval_ref_path"], "eval.json");
        assert_eq!(ctx["analysis_result_path"], "analysis.json");
        assert_eq!(ctx["trajectory_issues"][0]["issue_type"], "routing_policy");
    }

    #[test]
    fn plugin_registers_signals() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(SignalsPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let signals = ctx.service::<dyn Signals>(&SIGNALS).expect("signals seam");
        assert_eq!(signals.issue_type(&issue_with(json!({}))), "routing_policy");
        drop(effects);
        assert!(!ctx.has_service(&SIGNALS));
    }
}
