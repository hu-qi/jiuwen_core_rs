//! manifest seam:harness 元素的可序列化描述符目录与注册系统。
//!
//! 对齐 `openjiuwen/harness/manifest/*.py`(models/catalog/inputs/introspect/registration)。
//! 一个 `HarnessElementDescriptor` 描述一个 harness 元素(tool / rail / subagent):
//! kind、name、description、指向构造工厂的可反射引用(`factory_ref`)、构造输入的
//! JSON schema、以及实例暴露的公开接口方法列表。目录(catalog)按 name 登记描述符,
//! 注册(registration)把目录里的描述符按 kind 路由到对应的 provider 注册表。
//!
//! 契约零实现:目录存储/重复拒绝/路由判定由插件提供(ah-plugins-manifest)。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::effect::Effect;
use crate::seam::Seam;

/// 元素种类(描述符的注册目标)。
///
/// 对齐 Python `ElementKind`:TOOL → register_tool_provider,RAIL →
/// register_rail_provider,SUBAGENT → register_subagent_provider。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Tool,
    Rail,
    Subagent,
}

/// 构造实例暴露的一个公开方法或属性。
///
/// 对齐 Python `InterfaceMethod`(name / description / is_async)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InterfaceMethod {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_async: bool,
}

impl InterfaceMethod {
    /// 构造一个接口方法描述。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            is_async: false,
        }
    }

    /// 带描述与异步标记的构造。
    pub fn with_meta(
        name: impl Into<String>,
        description: impl Into<String>,
        is_async: bool,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            is_async,
        }
    }
}

/// 可序列化的 harness 元素描述符。
///
/// 对齐 Python `HarnessElementDescriptor`:
/// - `kind`:元素种类(tool / rail / subagent);
/// - `name`:唯一元素名,同时是 spec `type` / `factory_name`;
/// - `description`:元素用途的人类可读摘要;
/// - `factory_ref`:`"module:qualname"` 风格的构造工厂引用(entry-point 风格);
/// - `input_schema`:构造输入的 JSON schema(每个 property 带 source 标签);
/// - `input_model_ref`:构造输入模型的引用(None 表示无构造输入);
/// - `interface_methods`:构造实例公开的接口方法。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HarnessElementDescriptor {
    pub kind: ElementKind,
    pub name: String,
    pub description: String,
    pub factory_ref: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub input_model_ref: Option<String>,
    #[serde(default)]
    pub interface_methods: Vec<InterfaceMethod>,
}

/// 构造输入来源:spec `params` 字典或运行时 `BuildContext`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSource {
    Params,
    Context,
}

/// 一个构造输入字段的声明(spec 字段)。
///
/// 对齐 Python `param_field` / `context_field` 的元数据:
/// - `source=Params`:从 spec `params[name]` 读取(缺省用 default);
/// - `source=Context` + `context_attr`:直接 `getattr(context, attr)`;
/// - `source=Context` + `resolver_ref`:调用解析器 `(context) -> value`。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InputFieldSpec {
    pub name: String,
    pub source: InputSource,
    #[serde(default)]
    pub context_attr: Option<String>,
    #[serde(default)]
    pub resolver_ref: Option<String>,
    /// 字段默认值;None 表示无默认(字段在 resolve 中可选)。
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub description: String,
}

impl InputFieldSpec {
    /// 声明一个 params 来源的字段(`param_field`)。
    pub fn param_field(
        name: impl Into<String>,
        default: Option<Value>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: InputSource::Params,
            context_attr: None,
            resolver_ref: None,
            default,
            description: description.into(),
        }
    }

    /// 声明一个 context 来源的字段(`context_field`,按属性名读取)。
    pub fn context_attr_field(
        name: impl Into<String>,
        attr: impl Into<String>,
        default: Option<Value>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: InputSource::Context,
            context_attr: Some(attr.into()),
            resolver_ref: None,
            default,
            description: description.into(),
        }
    }

    /// 声明一个 context 来源的字段(`context_field`,按解析器引用读取)。
    pub fn context_resolver_field(
        name: impl Into<String>,
        resolver_ref: impl Into<String>,
        default: Option<Value>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: InputSource::Context,
            context_attr: None,
            resolver_ref: Some(resolver_ref.into()),
            default,
            description: description.into(),
        }
    }
}

/// 构造输入模型的声明(字段集合)。
///
/// 对齐 Python `ConstructionInput`:每个字段带 source 标签,`resolve` 从
/// `params + context` 提取并校验。`EmptyInput` 是空字段集。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConstructionInputModel {
    pub fields: Vec<InputFieldSpec>,
}

impl ConstructionInputModel {
    /// 空输入模型(元素不取构造输入)。
    pub fn empty() -> Self {
        Self { fields: Vec::new() }
    }

    /// 是否空输入模型。
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// 从声明字段生成 JSON schema(对齐 pydantic `model_json_schema` 的
    /// source 标签语义:每个 property 携带 `source` / `context_attr` /
    /// `resolver_ref` / `default` / `description`)。
    pub fn json_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        for field in &self.fields {
            let mut prop = serde_json::Map::new();
            if !field.description.is_empty() {
                prop.insert(
                    "description".to_string(),
                    Value::String(field.description.clone()),
                );
            }
            let source = match field.source {
                InputSource::Params => "params",
                InputSource::Context => "context",
            };
            prop.insert("source".to_string(), Value::String(source.to_string()));
            if let Some(attr) = &field.context_attr {
                prop.insert("context_attr".to_string(), Value::String(attr.clone()));
            }
            if let Some(resolver) = &field.resolver_ref {
                prop.insert("resolver_ref".to_string(), Value::String(resolver.clone()));
            }
            if let Some(default) = &field.default {
                prop.insert("default".to_string(), default.clone());
            }
            properties.insert(field.name.clone(), Value::Object(prop));
        }
        serde_json::json!({
            "type": "object",
            "properties": properties,
        })
    }
}

/// manifest 目录错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl core::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ManifestError {}

/// manifest 目录 Seam(Service Definition):描述符登记 / 查询 / JSON 导出。
pub trait ManifestCatalog: Seam {
    /// 登记一个描述符,拒绝重名。
    ///
    /// 返回可逆注册 guard:drop 即从目录移除。重名返回 `ManifestError`
    /// (`Duplicate harness element name: "name"`)。
    fn add_descriptor(&self, descriptor: HarnessElementDescriptor)
    -> Result<Effect, ManifestError>;

    /// 按名取描述符。
    fn get(&self, name: &str) -> Option<HarnessElementDescriptor>;

    /// 当前目录全部描述符(按登记顺序)。
    fn list(&self) -> Vec<HarnessElementDescriptor>;

    /// 导出为 JSON-ready 列表(`list_elements`)。
    fn list_elements(&self) -> Vec<Value>;
}

/// manifest 工厂注册表 Seam:`factory_ref` 字符串 → 工厂函数。
///
/// 对齐 Python `resolve_factory`:entry-point 风格引用解析到可调用对象。
/// Rust 侧以字符串引用为键,工厂在插件注册时按引用登记。
pub trait ManifestFactoryRegistry: Seam {
    /// 登记一个工厂,返回可逆注册 guard(drop 即反注册)。
    fn register(&self, factory_ref: &str, factory: Arc<dyn ElementFactory>) -> Effect;

    /// 按引用解析工厂;未登记返回 None。
    fn resolve(&self, factory_ref: &str) -> Option<Arc<dyn ElementFactory>>;

    /// 当前全部工厂引用(无序)。
    fn refs(&self) -> Vec<String>;
}

/// 元素构造工厂:`(params, context) -> 构造结果`。
///
/// 对齐 Python 的 `(params, context) -> instance` provider 契约。Rust 侧
/// 构造结果为序列化值(如 JSON 配置);真实实例的组装由消费方解释。
pub trait ElementFactory: Send + Sync {
    /// 用 spec params 与构建上下文构造元素。
    fn build(
        &self,
        params: &Value,
        context: &dyn ManifestContext,
        factories: &dyn ManifestFactoryRegistry,
    ) -> Result<Value, ManifestError>;
}

/// 构建上下文视图:鸭子类型属性访问。
///
/// 对齐 Python `BuildContext` 的属性读取(如 `language` / `member_card_id` /
/// `project_dir` / `workspace` / `extras`)。
pub trait ManifestContext: Send + Sync {
    /// 读取上下文属性;不存在返回 None。
    fn attr(&self, name: &str) -> Option<Value>;
}

/// manifest 注册 Seam:把目录描述符路由到 provider 注册表。
pub trait ManifestRegistration: Seam {
    /// 登记一个 tool provider(name 键,幂等覆盖),返回可逆注册 guard。
    fn register_tool_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect;

    /// 登记一个 rail provider(name 键,幂等覆盖),返回可逆注册 guard。
    fn register_rail_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect;

    /// 登记一个 subagent provider(name 键,幂等覆盖),返回可逆注册 guard。
    fn register_subagent_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect;

    /// 迭代目录描述符并注册到对应 provider 注册表(幂等)。
    ///
    /// TOOL → tool provider;RAIL → rail provider;SUBAGENT → subagent provider。
    /// 类 rail 的 builder 由插件用 class_rail_adapter 包装后注册。
    fn register_from_catalog(&self) -> Result<Vec<Effect>, ManifestError>;

    /// 取 tool provider 工厂。
    fn tool_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>>;

    /// 取 rail provider 工厂。
    fn rail_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>>;

    /// 取 subagent provider 工厂。
    fn subagent_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>>;
}

/// 便捷工具:构建一个可 JSON 序列化的上下文适配器。
pub struct MapManifestContext {
    attrs: HashMap<String, Value>,
}

impl MapManifestContext {
    /// 从属性映射构造上下文。
    pub fn new(attrs: HashMap<String, Value>) -> Self {
        Self { attrs }
    }

    /// 从键值对构造上下文。
    pub fn from_pairs(pairs: Vec<(&str, Value)>) -> Self {
        Self {
            attrs: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        }
    }
}

impl ManifestContext for MapManifestContext {
    fn attr(&self, name: &str) -> Option<Value> {
        self.attrs.get(name).cloned()
    }
}

/// 默认接口方法:按元素种类返回规范接口方法(对齐
/// `introspect.default_interface_methods`)。
pub fn default_interface_methods(kind: ElementKind) -> Vec<InterfaceMethod> {
    match kind {
        // Rails 暴露 AgentRail 生命周期钩子(公开、非下划线、排除内部 plumbing)。
        ElementKind::Rail => vec![
            InterfaceMethod::new("init"),
            InterfaceMethod::new("uninit"),
            InterfaceMethod::new("before_invoke"),
            InterfaceMethod::new("after_invoke"),
            InterfaceMethod::new("on_user_message"),
            InterfaceMethod::new("before_model_call"),
            InterfaceMethod::new("after_model_call"),
            InterfaceMethod::new("on_model_exception"),
            InterfaceMethod::new("before_tool_call"),
            InterfaceMethod::new("after_tool_call"),
            InterfaceMethod::new("on_tool_exception"),
            InterfaceMethod::new("before_task_iteration"),
            InterfaceMethod::new("after_task_iteration"),
        ],
        // Tools 暴露 Tool invoke / stream / card 表面。
        ElementKind::Tool => vec![
            InterfaceMethod::new("invoke"),
            InterfaceMethod::new("stream"),
            InterfaceMethod::new("card"),
        ],
        // Sub-agents 构造 SubAgentConfig spec 而非接口对象,暴露无。
        ElementKind::Subagent => Vec::new(),
    }
}

/// `"module:qualname"` 风格的工厂引用构造(`factory_ref`)。
pub fn factory_ref(module: &str, qualname: &str) -> String {
    format!("{module}:{qualname}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_json_round_trips() {
        let descriptor = HarnessElementDescriptor {
            kind: ElementKind::Rail,
            name: "test.sample".to_string(),
            description: "A sample element.".to_string(),
            factory_ref: "some.module:builder".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            input_model_ref: Some("some.module:SampleInput".to_string()),
            interface_methods: vec![InterfaceMethod {
                name: "run".to_string(),
                description: String::new(),
                is_async: true,
            }],
        };
        let json = serde_json::to_string(&descriptor).expect("serialize");
        let restored: HarnessElementDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, descriptor);
    }

    #[test]
    fn factory_ref_formats_module_colon_qualname() {
        assert_eq!(factory_ref("some.module", "builder"), "some.module:builder");
    }

    #[test]
    fn default_interface_methods_per_kind() {
        assert!(!default_interface_methods(ElementKind::Rail).is_empty());
        assert!(!default_interface_methods(ElementKind::Tool).is_empty());
        assert!(default_interface_methods(ElementKind::Subagent).is_empty());
    }

    #[test]
    fn input_schema_carries_source_tags() {
        let model = ConstructionInputModel {
            fields: vec![
                InputFieldSpec::param_field(
                    "tool_names",
                    Some(serde_json::json!(["switch_mode"])),
                    "Tool names from spec params.",
                ),
                InputFieldSpec::context_attr_field(
                    "language",
                    "language",
                    Some(serde_json::json!("cn")),
                    "Language read directly off the context.",
                ),
            ],
        };
        let schema = model.json_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["tool_names"]["source"], "params");
        assert_eq!(schema["properties"]["language"]["source"], "context");
        assert_eq!(schema["properties"]["language"]["context_attr"], "language");
        assert_eq!(
            schema["properties"]["tool_names"]["default"][0],
            "switch_mode"
        );
    }

    #[test]
    fn map_context_reads_attrs() {
        let ctx = MapManifestContext::from_pairs(vec![("language", serde_json::json!("fr"))]);
        assert_eq!(ctx.attr("language"), Some(serde_json::json!("fr")));
        assert_eq!(ctx.attr("missing"), None);
    }
}
