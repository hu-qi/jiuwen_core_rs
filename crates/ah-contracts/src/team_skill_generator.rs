//! team_skill_generator seam:团队技能生成的确定性归一化。
//!
//! 对齐 `openjiuwen/rsi/team_skill_generator/generator.py` 的确定性部分:
//! - 辅助:`slugify` / `single_line` / `string_list`;
//! - 归一化:`normalize_team_skill_plan`(team_name/description/roles/workflow_steps/
//!   acceptance_criteria,缺省与回退规则)/ `normalize_roles`(≥2 角色,id 去重,
//!   kind ∈ {ai_agent, human_agent},各字段缺省)/ `normalize_workflow_steps`
//!   (executor 校验:非 leader 须在 role_ids,否则回退 leader;空 → 默认两步);
//! - `write_skill_md` 骨架。
//!
//! 契约零实现:LLM plan/create/repair 调用由插件注入;本契约只定义纯函数。

use crate::seam::Seam;

/// slug 化(对齐 `_slugify`;命名 plan_slugify 避免与 skill_creator 冲突)。
pub fn plan_slugify(value: &str, default: &str) -> String {
    let text = value.trim().to_lowercase();
    let mut cleaned = String::new();
    let mut prev_dash = false;
    for c in text.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            cleaned.push(c);
            prev_dash = false;
        } else if !prev_dash {
            cleaned.push('-');
            prev_dash = true;
        }
    }
    let cleaned = cleaned.trim_matches('-').to_string();
    let truncated: String = cleaned.chars().take(80).collect();
    let truncated = truncated.trim_matches('-').to_string();
    if truncated.is_empty() {
        default.to_string()
    } else {
        truncated
    }
}

/// 单行化(对齐 `_single_line`):空白折叠 + trim,空回退 default。
pub fn single_line(value: &str, default: &str) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        default.to_string()
    } else {
        text
    }
}

/// 字符串列表(对齐 `_string_list`):字符串 → 单元素;非列表 → 空;逐项单行化。
pub fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let value = match value {
        Some(v) => v,
        None => return Vec::new(),
    };
    let items: Vec<&serde_json::Value> = match value {
        serde_json::Value::String(s) => {
            return {
                let text = single_line(s, "");
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text]
                }
            };
        }
        serde_json::Value::Array(a) => a.iter().collect(),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        let text = single_line(item.as_str().unwrap_or(""), "");
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}

/// 角色归一化错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillGenError(pub String);

impl core::fmt::Display for SkillGenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SkillGenError {}

/// 归一化角色(对齐 `_normalize_roles`)。
///
/// 非 dict 项跳过;id 重复 → Err;kind 非法 → ai_agent;缺省字段回退。
pub fn normalize_roles(
    raw_roles: Option<&serde_json::Value>,
) -> Result<Vec<serde_json::Value>, SkillGenError> {
    let raw_roles = match raw_roles {
        Some(serde_json::Value::Array(a)) => a,
        _ => {
            return Err(SkillGenError(
                "team_skill_plan.roles must be a non-empty list".to_string(),
            ));
        }
    };
    let mut roles: Vec<serde_json::Value> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (index, raw_role) in raw_roles.iter().enumerate() {
        let raw_role = match raw_role {
            serde_json::Value::Object(m) => m,
            _ => continue,
        };
        let role_id = plan_slugify(
            raw_role
                .get("id")
                .or_else(|| raw_role.get("role_id"))
                .or_else(|| raw_role.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            &format!("role-{}", index + 1),
        );
        if seen.contains(&role_id) {
            return Err(SkillGenError(format!(
                "duplicate Team Skill role id: {role_id}"
            )));
        }
        seen.insert(role_id.clone());
        let kind = raw_role
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("ai_agent")
            .trim();
        let kind = if kind == "ai_agent" || kind == "human_agent" {
            kind.to_string()
        } else {
            "ai_agent".to_string()
        };
        let purpose = single_line(
            raw_role
                .get("purpose")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            &format!("Perform the {role_id} role."),
        );
        let motto = single_line(
            raw_role.get("motto").and_then(|v| v.as_str()).unwrap_or(""),
            &format!("I own the {role_id} slice and make it concrete."),
        )
        .trim_matches(|c| c == '\'' || c == '"')
        .to_string();
        let responsibilities = string_list(raw_role.get("responsibilities"));
        let responsibilities = if responsibilities.is_empty() {
            vec![purpose.clone()]
        } else {
            responsibilities
        };
        let success_criteria = string_list(raw_role.get("success_criteria"));
        let success_criteria = if success_criteria.is_empty() {
            vec![format!("{role_id} produces an inspectable output.")]
        } else {
            success_criteria
        };
        let forbidden = string_list(raw_role.get("forbidden"));
        let forbidden = if forbidden.is_empty() {
            vec!["Do not take over another role's assigned output.".to_string()]
        } else {
            forbidden
        };
        let mandatory = string_list(raw_role.get("mandatory"));
        let mandatory = if mandatory.is_empty() {
            vec![format!(
                "You MUST produce concrete evidence for the {role_id} output."
            )]
        } else {
            mandatory
        };
        let output_sections = string_list(raw_role.get("output_sections"));
        let output_sections = if output_sections.is_empty() {
            vec![
                "Findings".to_string(),
                "Evidence".to_string(),
                "Output".to_string(),
            ]
        } else {
            output_sections
        };
        roles.push(serde_json::json!({
            "id": role_id,
            "kind": kind,
            "purpose": purpose,
            "motto": motto,
            "responsibilities": responsibilities,
            "success_criteria": success_criteria,
            "forbidden": forbidden,
            "mandatory": mandatory,
            "output_sections": output_sections,
            "skills": string_list(raw_role.get("skills")),
            "tools": string_list(raw_role.get("tools")),
        }));
    }
    if roles.is_empty() {
        return Err(SkillGenError(
            "team_skill_plan.roles did not contain any valid roles".to_string(),
        ));
    }
    Ok(roles)
}

/// 归一化工作流步骤(对齐 `_normalize_workflow_steps`)。
///
/// executor 非 leader 且不在 role_ids → 回退 leader;空 → 默认两步。
pub fn normalize_workflow_steps(
    raw_steps: Option<&serde_json::Value>,
    role_ids: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let mut steps: Vec<serde_json::Value> = Vec::new();
    if let Some(serde_json::Value::Array(items)) = raw_steps {
        for (index, raw_step) in items.iter().enumerate() {
            let raw_step = match raw_step {
                serde_json::Value::Object(m) => m,
                _ => continue,
            };
            let executor = single_line(
                raw_step
                    .get("executor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("leader"),
                "leader",
            );
            let executor = if executor != "leader" && !role_ids.contains(&executor) {
                "leader".to_string()
            } else {
                executor
            };
            steps.push(serde_json::json!({
                "name": single_line(raw_step.get("name").and_then(|v| v.as_str()).unwrap_or(""), &format!("Step {}", index + 1)),
                "executor": executor,
                "action": single_line(raw_step.get("action").and_then(|v| v.as_str()).unwrap_or(""), "Execute the assigned team workflow step."),
                "output": single_line(raw_step.get("output").and_then(|v| v.as_str()).unwrap_or(""), "Structured step output"),
                "quality_gate": single_line(raw_step.get("quality_gate").and_then(|v| v.as_str()).unwrap_or(""), "Output is concrete and can be inspected by the next step."),
            }));
        }
    }
    if !steps.is_empty() {
        return steps;
    }
    vec![
        serde_json::json!({
            "name": "Plan role work",
            "executor": "leader",
            "action": "Dispatch teammate roles with the task context and expected outputs.",
            "output": "Role assignments",
            "quality_gate": "Each role receives a distinct responsibility and output contract.",
        }),
        serde_json::json!({
            "name": "Integrate role outputs",
            "executor": "leader",
            "action": "Combine role outputs into the final task deliverable.",
            "output": "Final deliverable",
            "quality_gate": "The final deliverable satisfies the task request.",
        }),
    ]
}

/// 归一化团队技能计划(对齐 `_normalize_team_skill_plan`)。
pub fn normalize_team_skill_plan(
    raw_plan: &serde_json::Value,
    task: &str,
) -> Result<serde_json::Value, SkillGenError> {
    let plan = match raw_plan {
        serde_json::Value::Object(m) => m,
        _ => {
            return Err(SkillGenError(
                "Team Skill plan response must be a JSON object".to_string(),
            ));
        }
    };
    // team_skill_plan 或顶层。
    let plan = plan
        .get("team_skill_plan")
        .and_then(|v| v.as_object())
        .unwrap_or(plan);
    let default_team = format!("{}-team", plan_slugify(task, "generated-team"));
    let team_name = plan_slugify(
        plan.get("team_name")
            .or_else(|| plan.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        &default_team,
    );
    let description = single_line(
        plan.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        &format!("Team Skill for {task}"),
    );
    let roles = normalize_roles(plan.get("roles"))?;
    if roles.len() < 2 {
        return Err(SkillGenError(
            "Team Skill plan must include at least two teammate roles".to_string(),
        ));
    }
    let role_ids: std::collections::BTreeSet<String> = roles
        .iter()
        .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    let workflow_steps = normalize_workflow_steps(plan.get("workflow_steps"), &role_ids);
    let acceptance = string_list(plan.get("acceptance_criteria"));
    let acceptance = if acceptance.is_empty() {
        vec![
            "The final deliverable satisfies the original task request.".to_string(),
            "All role outputs needed by the workflow are integrated.".to_string(),
        ]
    } else {
        acceptance
    };
    Ok(serde_json::json!({
        "team_name": team_name,
        "description": description,
        "roles": roles,
        "workflow_steps": workflow_steps,
        "acceptance_criteria": acceptance,
    }))
}

/// SKILL.md 骨架(对齐 `_write_skill_md` 的确定性部分)。
pub fn write_skill_md(plan: &serde_json::Value) -> String {
    let team_name = plan
        .get("team_name")
        .and_then(|v| v.as_str())
        .unwrap_or("team");
    let description = plan
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut md = String::new();
    md.push_str(&format!(
        "---\nname: {team_name}\ndescription: {description}\n---\n\n"
    ));
    md.push_str(&format!("# {team_name}\n\n"));
    if let Some(roles) = plan.get("roles").and_then(|v| v.as_array()) {
        md.push_str("## Roles\n\n");
        for role in roles {
            if let Some(id) = role.get("id").and_then(|v| v.as_str()) {
                let purpose = role.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
                md.push_str(&format!("- **{id}**: {purpose}\n"));
            }
        }
        md.push('\n');
    }
    if let Some(steps) = plan.get("workflow_steps").and_then(|v| v.as_array()) {
        md.push_str("## Workflow\n\n");
        for (i, step) in steps.iter().enumerate() {
            let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let executor = step.get("executor").and_then(|v| v.as_str()).unwrap_or("");
            md.push_str(&format!("{}. **{name}** ({executor})\n", i + 1));
        }
        md.push('\n');
    }
    md
}

/// 团队技能生成 Seam(Service Definition):确定性归一化门面。
pub trait TeamSkillPlanNormalizer: Seam {
    /// 归一化 LLM 计划响应为结构化计划。
    fn normalize_plan(
        &self,
        raw_plan: &serde_json::Value,
        task: &str,
    ) -> Result<serde_json::Value, SkillGenError>;

    /// 生成 SKILL.md 骨架。
    fn build_skill_md(&self, plan: &serde_json::Value) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_normalizes() {
        assert_eq!(plan_slugify("My Team! 2", "t"), "my-team-2");
        assert_eq!(plan_slugify("---", "t"), "t");
        assert_eq!(plan_slugify("  UPPER  ", "t"), "upper");
    }

    #[test]
    fn single_line_collapses_whitespace() {
        assert_eq!(single_line("  a  b\n c  ", "d"), "a b c");
        assert_eq!(single_line("  ", "d"), "d");
    }

    #[test]
    fn string_list_variants() {
        assert_eq!(
            string_list(Some(&serde_json::json!("a b"))),
            vec!["a b".to_string()]
        );
        assert_eq!(
            string_list(Some(&serde_json::json!(["x", " y ", "  "]))),
            vec!["x".to_string(), "y".to_string()]
        );
        assert!(string_list(None).is_empty());
        assert!(string_list(Some(&serde_json::json!(42))).is_empty());
    }

    #[test]
    fn roles_normalize_with_defaults() {
        let raw = serde_json::json!([
            {"id": "researcher", "kind": "ai_agent", "purpose": "Research"},
            {"name": "coder", "kind": "bogus", "motto": " I code "},
        ]);
        let roles = normalize_roles(Some(&raw)).expect("roles");
        assert_eq!(roles.len(), 2);
        assert_eq!(roles[0]["id"], "researcher");
        assert_eq!(roles[1]["id"], "coder");
        // kind 非法 → ai_agent。
        assert_eq!(roles[1]["kind"], "ai_agent");
        // motto 去引号。
        assert_eq!(roles[1]["motto"], "I code");
        // 缺省 success_criteria。
        assert_eq!(
            roles[1]["success_criteria"][0],
            "coder produces an inspectable output."
        );
        // 缺省 output_sections。
        assert_eq!(roles[0]["output_sections"][0], "Findings");
    }

    #[test]
    fn roles_reject_duplicate_and_empty() {
        let dup = serde_json::json!([{"id": "r"}, {"id": "r"}]);
        let err = normalize_roles(Some(&dup)).expect_err("dup");
        assert!(err.0.contains("duplicate Team Skill role id"));

        let empty = serde_json::json!([]);
        let err2 = normalize_roles(Some(&empty)).expect_err("empty");
        assert!(err2.0.contains("did not contain any valid roles"));

        let non_list = serde_json::json!({});
        let err3 = normalize_roles(Some(&non_list)).expect_err("non-list");
        assert!(err3.0.contains("must be a non-empty list"));
    }

    #[test]
    fn workflow_steps_validate_executor_and_default() {
        let role_ids = std::collections::BTreeSet::from(["r1".to_string()]);
        let raw = serde_json::json!([
            {"name": "s1", "executor": "r1"},
            {"name": "s2", "executor": "ghost"},
        ]);
        let steps = normalize_workflow_steps(Some(&raw), &role_ids);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["executor"], "r1");
        // ghost 不在 role_ids → leader。
        assert_eq!(steps[1]["executor"], "leader");
        // 空 → 默认两步。
        let defaults = normalize_workflow_steps(None, &role_ids);
        assert_eq!(defaults.len(), 2);
        assert_eq!(defaults[0]["name"], "Plan role work");
    }

    #[test]
    fn plan_normalizes_with_fallbacks() {
        let raw = serde_json::json!({
            "team_skill_plan": {
                "team_name": "Data Team",
                "roles": [
                    {"id": "collector", "purpose": "Collect"},
                    {"id": "analyzer", "purpose": "Analyze"},
                ],
            }
        });
        let plan = normalize_team_skill_plan(&raw, "analyze data").expect("plan");
        assert_eq!(plan["team_name"], "data-team");
        assert_eq!(plan["roles"].as_array().unwrap().len(), 2);
        // 缺省 acceptance。
        assert_eq!(plan["acceptance_criteria"].as_array().unwrap().len(), 2);
        // 缺省 workflow → 默认两步。
        assert_eq!(plan["workflow_steps"].as_array().unwrap().len(), 2);
        // description 缺省 → 回退 task。
        assert_eq!(plan["description"], "Team Skill for analyze data");
    }

    #[test]
    fn plan_rejects_single_role() {
        let raw = serde_json::json!({
            "roles": [{"id": "only"}]
        });
        let err = normalize_team_skill_plan(&raw, "t").expect_err("single");
        assert!(err.0.contains("at least two teammate roles"));
    }

    #[test]
    fn skill_md_builds_skeleton() {
        let plan = serde_json::json!({
            "team_name": "data-team",
            "description": "Team Skill for data",
            "roles": [{"id": "collector", "purpose": "Collect"}],
            "workflow_steps": [{"name": "Plan role work", "executor": "leader"}],
        });
        let md = write_skill_md(&plan);
        assert!(md.contains("name: data-team"));
        assert!(md.contains("## Roles"));
        assert!(md.contains("**collector**"));
        assert!(md.contains("## Workflow"));
        assert!(md.contains("1. **Plan role work** (leader)"));
    }
}
