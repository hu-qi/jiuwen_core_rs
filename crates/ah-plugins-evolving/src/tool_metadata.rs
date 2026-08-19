//! 进化工具元数据提供器(对齐 prompts/tools/evolution.py)。
//!
//! 纯逻辑:6 个 canonical 进化工具的 subject schema / 描述 / 输入参数 JSON 构建 + 注册表查找。

use serde_json::{Value, json};

/// 语言文本选择(缺失回退中文,对齐 _text)。
pub fn text(language: &str, cn: &str, en: &str) -> String {
    if language == "en" {
        en.to_string()
    } else {
        cn.to_string()
    }
}

/// 值列表转逗号分隔串(对齐 _values)。
pub fn values(items: &[&str]) -> String {
    items.join(", ")
}

const SUPPORTED_KIND_ENUM: [&str; 2] = ["skill", "swarm-skill"];

/// 构建演进对象 subject schema(对齐 _subject_schema)。
pub fn build_evolution_subject_schema(language: &str) -> Value {
    let kind_desc = text(
        language,
        "演进对象类型。可选值：skill（常规技能）、swarm-skill（多智能体团队技能）。",
        "Evolution subject kind. Allowed values: skill, swarm-skill.",
    );
    let name_desc = text(language, "演进对象名称。", "Evolution subject name.");
    json!({
        "type": "object",
        "description": text(language, "演进目标对象。", "Evolution target object."),
        "properties": {
            "kind": {"type": "string", "enum": SUPPORTED_KIND_ENUM, "description": kind_desc},
            "name": {"type": "string", "description": name_desc},
        },
        "required": ["kind", "name"],
    })
}

/// 单个进化工具的名称/描述/输入参数(按名称分派;未知返回 None)。
pub fn tool_metadata(name: &str, language: &str) -> Option<(String, Value)> {
    let subject = build_evolution_subject_schema(language);
    match name {
        "prepare_skill_evolution" => Some((
            text(
                language,
                "在起草新经验前创建技能演进 review ref；用于用户同意将模糊反馈沉淀为 skill 演进后。",
                "Create a Skill evolution review ref before drafting new experiences. Use this after the user agrees to evolve a skill from ambiguous feedback.",
            ),
            json!({
                "type": "object",
                "properties": {
                    "subject": subject,
                    "user_intent": {"type": "string", "description": text(language, "用户已确认的演进意图或反馈摘要。", "User-approved evolution intent or feedback summary.")},
                    "user_confirmed": {"type": "boolean", "description": text(language, "用户明确同意演进该 skill 后必须为 true。", "Must be true after the user explicitly agrees to evolve this skill.")},
                },
                "required": ["subject", "user_confirmed"],
            }),
        )),
        "evolve_review_task" => Some((
            text(
                language,
                "为已准备的 evolution_review_ref 启动 evolution_reviewer subagent。",
                "Launch the evolution_reviewer subagent for a prepared evolution_review_ref.",
            ),
            json!({
                "type": "object",
                "properties": {
                    "evolution_review_ref": {"type": "string", "description": text(language, "prepare_skill_evolution 返回的 evolution_review_ref。", "Evolution review ref returned by prepare_skill_evolution.")},
                    "user_intent": {"type": "string", "description": text(language, "可选；用户已确认的演进意图或反馈摘要。", "Optional user-approved evolution intent or feedback summary.")},
                    "subject": subject,
                },
                "required": ["evolution_review_ref"],
            }),
        )),
        "list_skill_experiences" => {
            let targets = crate::protocols::EVOLUTION_TARGET_VALUES;
            let sections = crate::protocols::VALID_SECTIONS;
            Some((
                text(
                    language,
                    "查询已持久化的演进经验索引，用于聚焦查找或覆盖溢出索引；只返回元数据，不返回全文。",
                    "Query persisted evolution experience index entries for focused lookup or overflow coverage. Returns metadata only, not full content.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "subject": subject,
                        "min_score": {"type": "number", "description": text(language, "最低分数过滤。", "Minimum score filter.")},
                        "limit": {"type": "integer", "default": 20, "description": text(language, "最多返回的索引条目数。", "Maximum number of index items.")},
                        "cursor": {"type": "string", "description": text(language, "上一页溢出响应返回的游标。", "Cursor returned by a previous overflow response.")},
                        "target": {"type": "string", "enum": targets, "description": format!("{} 可选值：{}。", text(language, "结构化 target 过滤；", "Structured target filter. Allowed values: "), values(&crate::protocols::EVOLUTION_TARGET_VALUES))},
                        "section": {"type": "string", "enum": sections, "description": format!("{} 可选值：{}。", text(language, "结构化 section 过滤；", "Structured section filter. Allowed values: "), values(&crate::protocols::VALID_SECTIONS))},
                        "query": {"type": "string", "description": text(language, "仅在索引字段上匹配的字面量 OR 查询；多个词用 | 分隔，不是自然语言查询。", "Literal OR terms separated by | over index fields only; not natural language.")},
                        "sort": {"type": "string", "enum": ["score_desc", "updated_desc"], "default": "score_desc", "description": text(language, "索引排序方式。", "Index sort order.")},
                    },
                    "required": ["subject"],
                }),
            ))
        }
        "read_skill_experiences" => Some((
            text(
                language,
                "读取指定演进经验记录的全文内容。",
                "Read full content for selected evolution experience records.",
            ),
            json!({
                "type": "object",
                "properties": {
                    "subject": subject,
                    "record_ids": {"type": "array", "description": text(language, "要读取的经验记录 ID。", "Experience record IDs to read."), "items": {"type": "string"}},
                    "max_content_chars": {"type": "integer", "default": 2000, "description": text(language, "每条记录最多返回的内容字符数。", "Maximum content characters per record.")},
                },
                "required": ["subject", "record_ids"],
            }),
        )),
        "evolve_skill_experiences" => Some((
            text(
                language,
                "接受已完成 evolution review 中的已审查 Skill 演进 proposals。",
                "Accept reviewed Skill evolution proposals from a completed evolution review.",
            ),
            json!({
                "type": "object",
                "properties": {
                    "subject": subject,
                    "evolution_review_ref": {"type": "string", "description": text(language, "包含已审查 proposals 的已完成 Evolution Review Ref。", "Completed Evolution Review Ref containing reviewed proposals.")},
                    "selected_proposal_ids": {"type": "array", "description": text(language, "从已完成 review result 中选择要接受的 proposal_id 列表；不要复制或改写 proposal 正文。", "Proposal ids selected from the completed review result; do not copy or rewrite proposal content."), "items": {"type": "string"}},
                    "selection_reason": {"type": "string", "description": text(language, "可选；说明为什么接受这些 proposals，仅用于审计和审批摘要。", "Optional rationale for accepting these proposals, used for audit and approval summary.")},
                },
                "required": ["subject", "evolution_review_ref", "selected_proposal_ids"],
            }),
        )),
        "simplify_skill_experiences" => {
            let simplify = crate::protocols::SIMPLIFY_ACTION_VALUES;
            Some((
                text(
                    language,
                    "对已有演进经验执行删除、合并或改写动作。",
                    "Apply delete, merge, or refine actions to existing evolution experiences.",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "subject": subject,
                        "actions": {
                            "type": "array",
                            "description": text(language, "要执行的精简动作。", "Simplify actions to apply."),
                            "items": {
                                "type": "object",
                                "properties": {
                                    "action": {"type": "string", "enum": simplify, "description": format!("{} 可选值：{}。", text(language, "动作类型；", "Action type. Allowed values: "), values(&crate::protocols::SIMPLIFY_ACTION_VALUES))},
                                    "record_id": {"type": "string", "description": text(language, "主记录 ID。", "Primary record ID.")},
                                    "merge_remove_ids": {"type": "array", "description": text(language, "MERGE 时被移除的记录 ID。", "Record IDs removed during MERGE."), "items": {"type": "string"}},
                                    "new_content": {"type": "string", "description": text(language, "MERGE 或 REFINE 使用的替换内容。", "Replacement content for MERGE or REFINE.")},
                                    "reason": {"type": "string", "description": text(language, "执行该动作的原因。", "Reason for the action.")},
                                },
                                "required": ["action", "record_id"],
                            },
                        },
                    },
                    "required": ["subject", "actions"],
                }),
            ))
        }
        _ => None,
    }
}

/// 可用工具名(字母序,对齐 Python sorted(_REGISTRY.keys()))。
pub const TOOL_NAMES: [&str; 6] = [
    "evolve_review_task",
    "evolve_skill_experiences",
    "list_skill_experiences",
    "prepare_skill_evolution",
    "read_skill_experiences",
    "simplify_skill_experiences",
];

fn not_registered(name: &str) -> String {
    format!(
        "Evolution tool '{name}' not registered. Available: {}",
        values(&TOOL_NAMES)
    )
}

/// 按名称查工具描述;未知工具返回 Err(对齐 get_evolution_tool_description)。
pub fn get_evolution_tool_description(name: &str, language: &str) -> Result<String, String> {
    tool_metadata(name, language)
        .map(|(description, _)| description)
        .ok_or_else(|| not_registered(name))
}

/// 按名称查输入参数 schema;未知工具返回 Err(对齐 get_evolution_tool_input_params)。
pub fn get_evolution_tool_input_params(name: &str, language: &str) -> Result<Value, String> {
    tool_metadata(name, language)
        .map(|(_, params)| params)
        .ok_or_else(|| not_registered(name))
}

/// 工具卡片(id 规则:tool_id_agent_id 或 tool_id_<hex>;对齐 build_evolution_tool_card)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_params: Value,
}

fn unique_hex() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}{counter:04x}")
}

pub fn build_evolution_tool_card(
    name: &str,
    tool_id: &str,
    language: &str,
    agent_id: Option<&str>,
) -> ToolCard {
    let final_tool_id = match agent_id {
        Some(a) => format!("{tool_id}_{a}"),
        None => format!("{tool_id}_{}", unique_hex()),
    };
    ToolCard {
        id: final_tool_id,
        name: name.to_string(),
        description: get_evolution_tool_description(name, language).unwrap_or_default(),
        input_params: get_evolution_tool_input_params(name, language).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_schema_shape() {
        let cn = build_evolution_subject_schema("cn");
        let obj = cn.as_object().unwrap();
        assert_eq!(obj["type"], "object");
        let props = obj["properties"].as_object().unwrap();
        assert_eq!(props["kind"]["enum"], json!(["skill", "swarm-skill"]));
        assert_eq!(obj["required"], json!(["kind", "name"]));
        assert!(
            obj["description"]
                .as_str()
                .unwrap()
                .contains("演进目标对象")
        );
        let en = build_evolution_subject_schema("en");
        assert!(
            en["description"]
                .as_str()
                .unwrap()
                .contains("Evolution target object")
        );
    }

    #[test]
    fn all_tool_descriptions_resolve() {
        for name in TOOL_NAMES {
            let d = get_evolution_tool_description(name, "cn").unwrap();
            assert!(!d.is_empty(), "{name}");
            let den = get_evolution_tool_description(name, "en").unwrap();
            assert!(!den.is_empty(), "{name} en");
        }
    }

    #[test]
    fn unknown_tool_is_error_with_available() {
        let err = get_evolution_tool_description("nope", "cn").unwrap_err();
        assert!(err.contains("not registered"));
        assert!(err.contains("prepare_skill_evolution"));
        assert!(err.contains("simplify_skill_experiences"));
    }

    #[test]
    fn prepare_skill_evolution_requires_subject_and_confirmed() {
        let params = get_evolution_tool_input_params("prepare_skill_evolution", "cn").unwrap();
        assert_eq!(params["required"], json!(["subject", "user_confirmed"]));
        assert!(params["properties"]["user_confirmed"]["type"] == "boolean");
    }

    #[test]
    fn list_skill_experiences_enums_and_defaults() {
        let params = get_evolution_tool_input_params("list_skill_experiences", "cn").unwrap();
        let props = params["properties"].as_object().unwrap();
        assert_eq!(
            props["target"]["enum"],
            json!(["description", "body", "script"])
        );
        assert_eq!(props["limit"]["default"], 20);
        assert_eq!(props["sort"]["enum"], json!(["score_desc", "updated_desc"]));
        assert_eq!(props["sort"]["default"], "score_desc");
        assert_eq!(props["section"]["enum"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn read_skill_experiences_defaults() {
        let params = get_evolution_tool_input_params("read_skill_experiences", "cn").unwrap();
        assert_eq!(params["required"], json!(["subject", "record_ids"]));
        assert_eq!(params["properties"]["max_content_chars"]["default"], 2000);
    }

    #[test]
    fn simplify_actions_item_requires_action_and_record() {
        let params = get_evolution_tool_input_params("simplify_skill_experiences", "cn").unwrap();
        let item = &params["properties"]["actions"]["items"];
        assert_eq!(item["required"], json!(["action", "record_id"]));
        assert_eq!(
            item["properties"]["action"]["enum"],
            json!(["DELETE", "MERGE", "REFINE", "KEEP"]),
        );
    }

    #[test]
    fn tool_card_id_composition() {
        let card =
            build_evolution_tool_card("prepare_skill_evolution", "t1", "cn", Some("agent-7"));
        assert_eq!(card.id, "t1_agent-7");
        assert_eq!(card.name, "prepare_skill_evolution");
        assert!(!card.description.is_empty());
        let card2 = build_evolution_tool_card("list_skill_experiences", "t2", "en", None);
        assert!(card2.id.starts_with("t2_"));
        assert!(card2.id.len() > "t2_".len() + 8);
        assert!(card2.id != card.id);
    }
}
