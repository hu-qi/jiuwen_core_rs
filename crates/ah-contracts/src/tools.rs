//! tools seam:模型可见工具与工具注册表。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::effect::Effect;
use crate::seam::Seam;

/// 工具错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(pub String);

impl core::fmt::Display for ToolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

/// 模型可见工具(Service Definition)。
///
/// 实现方(插件)提供具体工具;消费方(agent 循环、workflow 组件)
/// 通过 [`ToolRegistry`] 按名调用,不 import 具体实现。
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// 工具稳定名称(模型可见,如 `echo`、`add`)。
    fn name(&self) -> &'static str;

    /// 工具描述(进入 prompt 组装)。
    fn description(&self) -> &'static str;

    /// 参数 JSON schema(进入模型请求的 tools 字段);默认宽松 object。
    fn parameters(&self) -> Value {
        json!({ "type": "object" })
    }

    /// 执行工具,返回 JSON 结果。
    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError>;

    /// 是否允许在崩溃恢复时重试同一调用。
    ///
    /// 默认 false,采取失败关闭策略。只有工具能以 `call_id` 去重时才可返回 true。
    fn idempotent(&self) -> bool {
        false
    }

    /// 携带持久化 tool-call id 执行,供幂等工具实现去重。
    async fn invoke_with_id(&self, _call_id: &str, arguments: Value) -> Result<Value, ToolError> {
        self.invoke(arguments).await
    }
}

/// 工具注册表 seam:工具集合的 Service Definition。
///
/// 消费方通过 `tools` 服务键解析注册表,再按名取工具/调用。
#[async_trait]
pub trait ToolRegistry: Seam {
    /// 注册一个工具,返回可逆注册 guard(drop 即移除)。
    fn register(&self, tool: Arc<dyn Tool>) -> Effect;

    /// 按名取工具。
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;

    /// 当前全部工具名(无序)。
    fn names(&self) -> Vec<String>;

    async fn invoke(&self, name: &str, arguments: Value) -> Result<Value, ToolError>;
    /// 按名调用工具。
    /// Execute a tool while preserving the stable call ID.
    async fn invoke_with_id(
        &self,
        name: &str,
        call_id: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        let _ = call_id;
        self.invoke(name, arguments).await
    }
}

// ------------------------------------------------------------------
// 工具执行管线事件(event-catalog: tools/pre-execute, tools/post-execute)
// ------------------------------------------------------------------

/// `tools/pre-execute` 事件:工具执行前发布(waterfall)。
///
/// 监听器(rails/鉴权/预算)可:
/// - 调用 Next::next 委托下游(可改写参数);
/// - 直接返回拒绝决策(短路,工具不执行)。
#[derive(Clone, Debug)]
pub struct ToolInvocation {
    pub name: String,
    pub arguments: Value,
}

impl crate::event::Event for ToolInvocation {
    const ID: &'static str = "tools/pre-execute";
}

/// `tools/pre-execute` waterfall 的决策值。
#[derive(Clone, Debug)]
pub struct ToolDecision {
    /// 是否允许执行。
    pub allow: bool,
    /// 拒绝原因(allow=false 时必填)。
    pub reason: Option<String>,
    /// 实际执行的参数(监听器可改写)。
    pub arguments: Value,
}

impl ToolDecision {
    /// 允许执行。
    pub fn allow(arguments: Value) -> Self {
        Self {
            allow: true,
            reason: None,
            arguments,
        }
    }

    /// 拒绝执行。
    pub fn deny(arguments: Value, reason: impl Into<String>) -> Self {
        Self {
            allow: false,
            reason: Some(reason.into()),
            arguments,
        }
    }
}

/// `tools/post-execute` 事件:工具执行完成后发布(serial)。
#[derive(Clone, Debug)]
pub struct ToolExecuted {
    pub name: String,
    pub arguments: Value,
    pub output: Value,
    pub elapsed_ms: u64,
}

impl crate::event::Event for ToolExecuted {
    const ID: &'static str = "tools/post-execute";
}
// ---------------------------------------------------------------------------
// ToolMetadataProvider seam(对齐 harness/prompts/tools/base.py)
// ---------------------------------------------------------------------------

/// 工具元数据(双语描述 + 参数 schema)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description_cn: String,
    pub description_en: String,
    pub params_cn: Value,
    pub params_en: Value,
    pub idempotent: bool,
}

/// 工具元数据校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMetadataError(pub String);

impl core::fmt::Display for ToolMetadataError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolMetadataError {}

/// 校验工具元数据的双语完整性(对齐 validate_provider):
/// - cn/en description 都非空;
/// - cn/en schema 都是 object(type=object + properties + required);
/// - cn/en schema 的 properties key 集合一致;
/// - 递归校验嵌套 properties/items 的 description。
pub fn validate_provider(meta: &ToolMetadata) -> Result<(), ToolMetadataError> {
    let name = &meta.name;
    if meta.description_cn.trim().is_empty() {
        return Err(ToolMetadataError(format!(
            "[{name}] cn description is empty"
        )));
    }
    if meta.description_en.trim().is_empty() {
        return Err(ToolMetadataError(format!(
            "[{name}] en description is empty"
        )));
    }
    validate_schema_pair(name, &meta.params_cn, &meta.params_en, "cn", "en", "")
}

fn validate_schema_pair(
    name: &str,
    ref_schema: &Value,
    other_schema: &Value,
    ref_lang: &str,
    other_lang: &str,
    path: &str,
) -> Result<(), ToolMetadataError> {
    let prefix = format!("[{name}]{path}");
    let check_schema = |schema: &Value, lang: &str| -> Result<(), ToolMetadataError> {
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(ToolMetadataError(format!(
                "{prefix} {lang} schema type != 'object'"
            )));
        }
        if schema.get("properties").is_none() {
            return Err(ToolMetadataError(format!(
                "{prefix} {lang} schema missing 'properties'"
            )));
        }
        if schema.get("required").is_none() {
            return Err(ToolMetadataError(format!(
                "{prefix} {lang} schema missing 'required'"
            )));
        }
        Ok(())
    };
    check_schema(ref_schema, ref_lang)?;
    check_schema(other_schema, other_lang)?;

    let ref_props = ref_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let other_props = other_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let ref_keys: std::collections::BTreeSet<String> = ref_props.keys().cloned().collect();
    let other_keys: std::collections::BTreeSet<String> = other_props.keys().cloned().collect();
    if ref_keys != other_keys {
        return Err(ToolMetadataError(format!(
            "{prefix} property keys differ: {ref_lang}={:?}, {other_lang}={:?}",
            ref_keys.iter().collect::<Vec<_>>(),
            other_keys.iter().collect::<Vec<_>>()
        )));
    }

    for key in &ref_keys {
        for (lang, props) in [(ref_lang, &ref_props), (other_lang, &other_props)] {
            let prop = props.get(key).unwrap();
            if prop.get("description").is_none() {
                return Err(ToolMetadataError(format!(
                    "{prefix}.{key} {lang} missing description"
                )));
            }
            if prop.get("type").and_then(Value::as_str) == Some("object")
                && prop.get("properties").is_some()
            {
                let other = if lang == ref_lang {
                    &other_props
                } else {
                    &ref_props
                };
                let other_prop = other.get(key).unwrap();
                validate_schema_pair(
                    name,
                    prop,
                    other_prop,
                    ref_lang,
                    other_lang,
                    &format!("{path}.{key}"),
                )?;
            }
            if prop.get("type").and_then(Value::as_str) == Some("array")
                && prop.get("items").is_some()
            {
                let other = if lang == ref_lang {
                    &other_props
                } else {
                    &ref_props
                };
                let other_items = other
                    .get(key)
                    .and_then(|o| o.get("items"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let items = prop.get("items").cloned().unwrap_or(Value::Null);
                if items.get("type").and_then(Value::as_str) == Some("object")
                    && items.get("properties").is_some()
                {
                    validate_schema_pair(
                        name,
                        &items,
                        &other_items,
                        ref_lang,
                        other_lang,
                        &format!("{path}.{key}[]"),
                    )?;
                }
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn valid_meta() -> ToolMetadata {
        ToolMetadata {
            name: "echo".to_string(),
            description_cn: "回显".to_string(),
            description_en: "Echo".to_string(),
            params_cn: json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "内容"}},
                "required": ["text"]
            }),
            params_en: json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "Content"}},
                "required": ["text"]
            }),
            idempotent: false,
        }
    }

    #[test]
    fn validate_accepts_valid_bilingual_meta() {
        assert!(validate_provider(&valid_meta()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_description() {
        let mut m = valid_meta();
        m.description_cn = "".to_string();
        let err = validate_provider(&m).unwrap_err();
        assert!(err.0.contains("cn description is empty"));
    }

    #[test]
    fn validate_rejects_non_object_schema() {
        let mut m = valid_meta();
        m.params_cn = json!({"type": "string"});
        let err = validate_provider(&m).unwrap_err();
        assert!(err.0.contains("type != 'object'"));
    }

    #[test]
    fn validate_rejects_missing_properties_or_required() {
        let mut m = valid_meta();
        m.params_cn = json!({"type": "object", "properties": {}});
        let err = validate_provider(&m).unwrap_err();
        assert!(err.0.contains("missing 'required'"));
    }

    #[test]
    fn validate_rejects_property_key_mismatch() {
        let mut m = valid_meta();
        m.params_en = json!({
            "type": "object",
            "properties": {"text": {"type": "string", "description": "X"}, "extra": {"type": "string", "description": "Y"}},
            "required": ["text"]
        });
        let err = validate_provider(&m).unwrap_err();
        assert!(err.0.contains("property keys differ"));
    }

    #[test]
    fn validate_rejects_missing_property_description() {
        let mut m = valid_meta();
        m.params_cn = json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"]
        });
        let err = validate_provider(&m).unwrap_err();
        assert!(err.0.contains("missing description"));
    }

    #[test]
    fn validate_nested_object_and_array() {
        let mk = |d: &str| {
            json!({
                "type": "object",
                "properties": {
                    "outer": {
                        "type": "object",
                        "description": "外层",
                        "properties": {"inner": {"type": "string", "description": d}},
                        "required": ["inner"],
                    },
                    "list": {
                        "type": "array",
                        "description": "列表",
                        "items": {"type": "object", "description": "项", "properties": {"v": {"type": "string", "description": d}}, "required": ["v"]},
                    },
                },
                "required": ["outer", "list"],
            })
        };
        let m = ToolMetadata {
            name: "nested".to_string(),
            description_cn: "嵌套".to_string(),
            description_en: "Nested".to_string(),
            params_cn: mk("中"),
            params_en: mk("En"),
            idempotent: false,
        };
        assert!(validate_provider(&m).is_ok());
    }
}
