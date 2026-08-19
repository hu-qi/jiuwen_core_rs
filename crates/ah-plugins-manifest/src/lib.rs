//! # ah-plugins-manifest
//!
//! 真实 harness 元素 manifest(1:1 对齐 `openjiuwen/harness/manifest/*.py`):
//! - **catalog**:描述符目录(按 name 登记,拒绝重名,JSON 导出);
//! - **factory registry**:`factory_ref` 字符串 → 工厂函数的可逆注册表;
//! - **registration**:把目录描述符按 kind 路由到 tool / rail / subagent
//!   provider 注册表(类 builder 经 class 适配器统一注册)。
//!
//! 契约零实现:类型与 seam 定义在 ah-contracts;本插件提供真实存储与判定。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::{MANIFEST, MANIFEST_FACTORIES, MANIFEST_REGISTRATION};
use ah_contracts::manifest::{
    ConstructionInputModel, ElementFactory, ElementKind, HarnessElementDescriptor, ManifestCatalog,
    ManifestContext, ManifestError, ManifestFactoryRegistry, ManifestRegistration,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 真实描述符目录:name → descriptor(登记顺序由 Vec 保持)。
pub struct Catalog {
    inner: Arc<Mutex<HashMap<String, HarnessElementDescriptor>>>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

impl Catalog {
    /// 构造空目录。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Seam for Catalog {}

impl ManifestCatalog for Catalog {
    fn add_descriptor(
        &self,
        descriptor: HarnessElementDescriptor,
    ) -> Result<Effect, ManifestError> {
        let name = descriptor.name.clone();
        let mut map = self.inner.lock().expect("catalog lock");
        if map.contains_key(&name) {
            return Err(ManifestError(format!(
                "Duplicate harness element name: {:?}",
                name
            )));
        }
        map.insert(name.clone(), descriptor);
        let inner = self.inner.clone();
        Ok(Effect::new(move || {
            let mut map = inner.lock().expect("catalog lock");
            map.remove(&name);
        }))
    }

    fn get(&self, name: &str) -> Option<HarnessElementDescriptor> {
        self.inner.lock().expect("catalog lock").get(name).cloned()
    }

    fn list(&self) -> Vec<HarnessElementDescriptor> {
        self.inner
            .lock()
            .expect("catalog lock")
            .values()
            .cloned()
            .collect()
    }

    fn list_elements(&self) -> Vec<Value> {
        self.inner
            .lock()
            .expect("catalog lock")
            .values()
            .map(|d| serde_json::to_value(d).expect("descriptor serializable"))
            .collect()
    }
}

/// 真实工厂注册表:factory_ref → 工厂。
pub struct FactoryRegistry {
    inner: Arc<Mutex<HashMap<String, Arc<dyn ElementFactory>>>>,
}

impl Default for FactoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FactoryRegistry {
    /// 构造空注册表。
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Seam for FactoryRegistry {}

impl ManifestFactoryRegistry for FactoryRegistry {
    fn register(&self, factory_ref: &str, factory: Arc<dyn ElementFactory>) -> Effect {
        let key = factory_ref.to_string();
        let mut map = self.inner.lock().expect("factory lock");
        map.insert(key.clone(), factory);
        let inner = self.inner.clone();
        Effect::new(move || {
            let mut map = inner.lock().expect("factory lock");
            map.remove(&key);
        })
    }

    fn resolve(&self, factory_ref: &str) -> Option<Arc<dyn ElementFactory>> {
        self.inner
            .lock()
            .expect("factory lock")
            .get(factory_ref)
            .cloned()
    }

    fn refs(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("factory lock")
            .keys()
            .cloned()
            .collect()
    }
}

/// 真实注册:kind 路由到 provider 注册表 + 目录驱动注册。
pub struct Registration {
    catalog: Arc<dyn ManifestCatalog>,
    factories: Arc<dyn ManifestFactoryRegistry>,
    tools: Arc<Mutex<HashMap<String, Arc<dyn ElementFactory>>>>,
    rails: Arc<Mutex<HashMap<String, Arc<dyn ElementFactory>>>>,
    subagents: Arc<Mutex<HashMap<String, Arc<dyn ElementFactory>>>>,
}

impl Registration {
    /// 构造注册器(注入目录与工厂注册表)。
    pub fn new(
        catalog: Arc<dyn ManifestCatalog>,
        factories: Arc<dyn ManifestFactoryRegistry>,
    ) -> Self {
        Self {
            catalog,
            factories,
            tools: Arc::new(Mutex::new(HashMap::new())),
            rails: Arc::new(Mutex::new(HashMap::new())),
            subagents: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Seam for Registration {}

impl ManifestRegistration for Registration {
    fn register_tool_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect {
        let key = name.to_string();
        let mut map = self.tools.lock().expect("tools lock");
        map.insert(key.clone(), factory);
        let inner = self.tools.clone();
        Effect::new(move || {
            let mut map = inner.lock().expect("tools lock");
            map.remove(&key);
        })
    }

    fn register_rail_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect {
        let key = name.to_string();
        let mut map = self.rails.lock().expect("rails lock");
        map.insert(key.clone(), factory);
        let inner = self.rails.clone();
        Effect::new(move || {
            let mut map = inner.lock().expect("rails lock");
            map.remove(&key);
        })
    }

    fn register_subagent_provider(&self, name: &str, factory: Arc<dyn ElementFactory>) -> Effect {
        let key = name.to_string();
        let mut map = self.subagents.lock().expect("subagents lock");
        map.insert(key.clone(), factory);
        let inner = self.subagents.clone();
        Effect::new(move || {
            let mut map = inner.lock().expect("subagents lock");
            map.remove(&key);
        })
    }

    fn register_from_catalog(&self) -> Result<Vec<Effect>, ManifestError> {
        let mut effects = Vec::new();
        for descriptor in self.catalog.list() {
            let factory = self
                .factories
                .resolve(&descriptor.factory_ref)
                .ok_or_else(|| {
                    ManifestError(format!(
                        "Cannot resolve factory for harness element {:?}: {:?}",
                        descriptor.name, descriptor.factory_ref
                    ))
                })?;
            let effect = match descriptor.kind {
                ElementKind::Tool => self.register_tool_provider(&descriptor.name, factory),
                ElementKind::Rail => self.register_rail_provider(&descriptor.name, factory),
                ElementKind::Subagent => self.register_subagent_provider(&descriptor.name, factory),
            };
            effects.push(effect);
        }
        Ok(effects)
    }

    fn tool_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>> {
        self.tools.lock().expect("tools lock").get(name).cloned()
    }

    fn rail_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>> {
        self.rails.lock().expect("rails lock").get(name).cloned()
    }

    fn subagent_provider(&self, name: &str) -> Option<Arc<dyn ElementFactory>> {
        self.subagents
            .lock()
            .expect("subagents lock")
            .get(name)
            .cloned()
    }
}

/// manifest 插件:注册 catalog + factory registry + registration 三个 seam。
pub struct ManifestPlugin;

impl Plugin for ManifestPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-manifest"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MANIFEST, MANIFEST_FACTORIES, MANIFEST_REGISTRATION]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let catalog: Arc<dyn ManifestCatalog> = Arc::new(Catalog::new());
        let factories: Arc<dyn ManifestFactoryRegistry> = Arc::new(FactoryRegistry::new());
        let registration: Arc<dyn ManifestRegistration> =
            Arc::new(Registration::new(catalog.clone(), factories.clone()));
        Ok(vec![
            ctx.register(MANIFEST, catalog),
            ctx.register(MANIFEST_FACTORIES, factories),
            ctx.register(MANIFEST_REGISTRATION, registration),
        ])
    }
}

/// 便捷构造:由 JSON 参数构建一个元素的工厂(基于 ConstructionInputModel 声明)。
///
/// 对齐 Python `ConstructionInput.resolve`:params 来源字段读 `params[name]`
/// (缺省用 default);context 来源字段读属性或调用解析器(None 结果丢弃,default
/// 生效);解析器按 `resolver_ref` 从工厂注册表解析。
pub struct ModelElementFactory {
    input: ConstructionInputModel,
}

impl ModelElementFactory {
    /// 构造:输入模型 + 构造结果工厂引用。
    pub fn new(input: ConstructionInputModel, _factory_ref: String) -> Self {
        Self { input }
    }
}

impl Seam for ModelElementFactory {}

impl ElementFactory for ModelElementFactory {
    fn build(
        &self,
        params: &Value,
        context: &dyn ManifestContext,
        factories: &dyn ManifestFactoryRegistry,
    ) -> Result<Value, ManifestError> {
        let mut resolved = serde_json::Map::new();
        for field in &self.input.fields {
            let value: Option<Value> = match field.source {
                ah_contracts::manifest::InputSource::Params => match params.get(&field.name) {
                    Some(v) => Some(v.clone()),
                    None => field.default.clone(),
                },
                ah_contracts::manifest::InputSource::Context => {
                    if let Some(resolver_ref) = &field.resolver_ref {
                        let resolver = factories.resolve(resolver_ref).ok_or_else(|| {
                            ManifestError(format!(
                                "Cannot resolve context resolver {:?} for field {:?}",
                                resolver_ref, field.name
                            ))
                        })?;
                        let resolved_value =
                            resolver.build(&serde_json::json!({}), context, factories)?;
                        if resolved_value.is_null() {
                            field.default.clone()
                        } else {
                            Some(resolved_value)
                        }
                    } else if let Some(attr) = &field.context_attr {
                        match context.attr(attr) {
                            Some(v) => Some(v),
                            None => field.default.clone(),
                        }
                    } else {
                        field.default.clone()
                    }
                }
            };
            if let Some(v) = value {
                resolved.insert(field.name.clone(), v);
            }
        }
        Ok(serde_json::Value::Object(resolved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::manifest::{
        InputFieldSpec, InputSource, InterfaceMethod, MapManifestContext,
    };
    use ah_contracts::prelude::default_interface_methods;
    use ah_hub::plugin::DynPlugin;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(ManifestPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    /// 反射式工具工厂:返回 marker dict。
    #[derive(Clone)]
    struct FakeToolFactory {
        kind: &'static str,
    }

    impl Seam for FakeToolFactory {}

    impl ElementFactory for FakeToolFactory {
        fn build(
            &self,
            params: &Value,
            _context: &dyn ManifestContext,
            _factories: &dyn ManifestFactoryRegistry,
        ) -> Result<Value, ManifestError> {
            Ok(serde_json::json!({
                "kind": self.kind,
                "params": params.clone(),
            }))
        }
    }

    #[test]
    fn add_descriptor_rejects_duplicates() {
        let (ctx, effects) = build_ctx();
        let catalog = ctx
            .service::<dyn ManifestCatalog>(&MANIFEST)
            .expect("catalog");
        let d = HarnessElementDescriptor {
            kind: ElementKind::Tool,
            name: "test.dup".to_string(),
            description: "first".to_string(),
            factory_ref: "mod:build_tool".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        };
        let _g = catalog.add_descriptor(d).expect("add");
        let dup = HarnessElementDescriptor {
            kind: ElementKind::Tool,
            name: "test.dup".to_string(),
            description: "second".to_string(),
            factory_ref: "mod:build_other".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        };
        let err = catalog.add_descriptor(dup).expect_err("duplicate rejected");
        assert!(err.0.contains("Duplicate harness element name"));
        drop(effects);
    }

    #[test]
    fn effect_removes_descriptor_on_drop() {
        let (ctx, _effects) = build_ctx();
        let catalog = ctx
            .service::<dyn ManifestCatalog>(&MANIFEST)
            .expect("catalog");
        let d = HarnessElementDescriptor {
            kind: ElementKind::Rail,
            name: "test.tmp".to_string(),
            description: "temporary".to_string(),
            factory_ref: "mod:builder".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        };
        {
            let _g = catalog.add_descriptor(d).expect("add");
            assert!(catalog.get("test.tmp").is_some());
        }
        assert!(catalog.get("test.tmp").is_none());
    }

    #[test]
    fn list_elements_is_json_serializable() {
        let (ctx, effects) = build_ctx();
        let catalog = ctx
            .service::<dyn ManifestCatalog>(&MANIFEST)
            .expect("catalog");
        let d = HarnessElementDescriptor {
            kind: ElementKind::Tool,
            name: "test.tool.json".to_string(),
            description: "tool".to_string(),
            factory_ref: "mod:build_tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            input_model_ref: Some("mod:SampleInput".to_string()),
            interface_methods: vec![InterfaceMethod::with_meta("invoke", "", true)],
        };
        let _g = catalog.add_descriptor(d).expect("add");
        let elements = catalog.list_elements();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0]["name"], "test.tool.json");
        assert_eq!(elements[0]["input_model_ref"], "mod:SampleInput");
        // 可 JSON 往返。
        let round = serde_json::to_string(&elements).expect("serialize");
        let _back: Vec<Value> = serde_json::from_str(&round).expect("deserialize");
        drop(effects);
    }

    #[test]
    fn register_from_catalog_routes_by_kind() {
        let (ctx, effects) = build_ctx();
        let catalog = ctx
            .service::<dyn ManifestCatalog>(&MANIFEST)
            .expect("catalog");
        let factories = ctx
            .service::<dyn ManifestFactoryRegistry>(&MANIFEST_FACTORIES)
            .expect("factories");
        let registration = ctx
            .service::<dyn ManifestRegistration>(&MANIFEST_REGISTRATION)
            .expect("registration");

        let tool = Arc::new(FakeToolFactory { kind: "tool" }) as Arc<dyn ElementFactory>;
        let rail = Arc::new(FakeToolFactory { kind: "rail" }) as Arc<dyn ElementFactory>;
        let sub = Arc::new(FakeToolFactory { kind: "subagent" }) as Arc<dyn ElementFactory>;
        let _f1 = factories.register("mod:build_tool", tool);
        let _f2 = factories.register("mod:build_rail", rail);
        let _f3 = factories.register("mod:build_subagent", sub);

        let mut add_effects = Vec::new();
        for (name, kind, factory_ref) in [
            ("test.tool", ElementKind::Tool, "mod:build_tool"),
            ("test.rail", ElementKind::Rail, "mod:build_rail"),
            ("test.subagent", ElementKind::Subagent, "mod:build_subagent"),
        ] {
            let d = HarnessElementDescriptor {
                kind,
                name: name.to_string(),
                description: String::new(),
                factory_ref: factory_ref.to_string(),
                input_schema: Value::Object(Default::default()),
                input_model_ref: None,
                interface_methods: Vec::new(),
            };
            let g = catalog.add_descriptor(d).expect("add");
            add_effects.push(g);
        }

        let _reg_effects = registration.register_from_catalog().expect("register all");
        assert!(registration.tool_provider("test.tool").is_some());
        assert!(registration.rail_provider("test.rail").is_some());
        assert!(registration.subagent_provider("test.subagent").is_some());
        drop(add_effects);
        drop(effects);
    }

    #[test]
    fn missing_factory_is_explicit_error() {
        let (ctx, effects) = build_ctx();
        let catalog = ctx
            .service::<dyn ManifestCatalog>(&MANIFEST)
            .expect("catalog");
        let registration = ctx
            .service::<dyn ManifestRegistration>(&MANIFEST_REGISTRATION)
            .expect("registration");
        let d = HarnessElementDescriptor {
            kind: ElementKind::Tool,
            name: "test.ghost".to_string(),
            description: String::new(),
            factory_ref: "mod:does_not_exist".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        };
        let _g = catalog.add_descriptor(d).expect("add");
        let err = registration
            .register_from_catalog()
            .expect_err("explicit error");
        assert!(err.0.contains("Cannot resolve factory"));
        drop(effects);
    }

    #[test]
    fn model_factory_resolves_each_source() {
        let input = ConstructionInputModel {
            fields: vec![
                InputFieldSpec {
                    name: "tool_names".to_string(),
                    source: InputSource::Params,
                    context_attr: None,
                    resolver_ref: None,
                    default: Some(serde_json::json!(["switch_mode"])),
                    description: "Tool names from spec params.".to_string(),
                },
                InputFieldSpec {
                    name: "language".to_string(),
                    source: InputSource::Context,
                    context_attr: Some("language".to_string()),
                    resolver_ref: None,
                    default: Some(serde_json::json!("cn")),
                    description: "Language read directly off the context.".to_string(),
                },
            ],
        };
        let (ctx, effects) = build_ctx();
        let factories = ctx
            .service::<dyn ManifestFactoryRegistry>(&MANIFEST_FACTORIES)
            .expect("factories");
        let factory = ModelElementFactory::new(input, "mod:builder".to_string());
        let context = MapManifestContext::from_pairs(vec![("language", serde_json::json!("fr"))]);
        let built = factory
            .build(
                &serde_json::json!({"tool_names": ["a", "b"]}),
                &context,
                &*factories,
            )
            .expect("build");
        assert_eq!(built["tool_names"][0], "a");
        assert_eq!(built["language"], "fr");
        drop(effects);
    }

    #[test]
    fn model_factory_falls_back_to_defaults() {
        let input = ConstructionInputModel {
            fields: vec![
                InputFieldSpec {
                    name: "tool_names".to_string(),
                    source: InputSource::Params,
                    context_attr: None,
                    resolver_ref: None,
                    default: Some(serde_json::json!(["switch_mode"])),
                    description: String::new(),
                },
                InputFieldSpec {
                    name: "language".to_string(),
                    source: InputSource::Context,
                    context_attr: Some("language".to_string()),
                    resolver_ref: None,
                    default: Some(serde_json::json!("cn")),
                    description: String::new(),
                },
            ],
        };
        let (ctx, effects) = build_ctx();
        let factories = ctx
            .service::<dyn ManifestFactoryRegistry>(&MANIFEST_FACTORIES)
            .expect("factories");
        let factory = ModelElementFactory::new(input, "mod:builder".to_string());
        let context = MapManifestContext::from_pairs(vec![]);
        let built = factory
            .build(&serde_json::json!({}), &context, &*factories)
            .expect("build");
        assert_eq!(built["tool_names"][0], "switch_mode");
        assert_eq!(built["language"], "cn");
        drop(effects);
    }

    #[test]
    fn kind_default_interface_methods() {
        assert!(!default_interface_methods(ElementKind::Rail).is_empty());
        assert!(!default_interface_methods(ElementKind::Tool).is_empty());
        assert!(default_interface_methods(ElementKind::Subagent).is_empty());
    }
}
