//! manifest 真实路径集成测试:挂载 ah-plugins-manifest 后,描述符目录 /
//! 工厂注册表 / kind 路由注册全链路真实执行(非 mock)。

use std::sync::Arc;

use ah_contracts::keys::{MANIFEST, MANIFEST_FACTORIES, MANIFEST_REGISTRATION};
use ah_contracts::manifest::{
    ConstructionInputModel, ElementFactory, ElementKind, HarnessElementDescriptor, ManifestCatalog,
    ManifestContext, ManifestError, ManifestFactoryRegistry, ManifestRegistration,
    MapManifestContext,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use serde_json::Value;

fn mount(ctx: &Context) -> Vec<Effect> {
    let plugins: Vec<DynPlugin> = vec![Arc::new(ah_plugins_manifest::ManifestPlugin)];
    ctx.mount_all(plugins).expect("mount manifest")
}

/// 反射式 builder:返回 marker JSON(对齐 Python 测试的 `_build_fake_tool`)。
struct MarkerFactory {
    kind: &'static str,
}

impl Seam for MarkerFactory {}

impl ElementFactory for MarkerFactory {
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
fn manifest_mount_provides_all_three_seams() {
    let ctx = Context::new();
    let effects = mount(&ctx);
    assert!(ctx.service::<dyn ManifestCatalog>(&MANIFEST).is_some());
    assert!(
        ctx.service::<dyn ManifestFactoryRegistry>(&MANIFEST_FACTORIES)
            .is_some()
    );
    assert!(
        ctx.service::<dyn ManifestRegistration>(&MANIFEST_REGISTRATION)
            .is_some()
    );
    drop(effects);
}

#[test]
fn full_chain_declare_resolve_register() {
    let ctx = Context::new();
    let effects = mount(&ctx);
    let catalog = ctx
        .service::<dyn ManifestCatalog>(&MANIFEST)
        .expect("catalog");
    let factories = ctx
        .service::<dyn ManifestFactoryRegistry>(&MANIFEST_FACTORIES)
        .expect("factories");
    let registration = ctx
        .service::<dyn ManifestRegistration>(&MANIFEST_REGISTRATION)
        .expect("registration");

    // 1) 登记工厂(factory_ref ↔ 工厂)。
    let factory: Arc<dyn ElementFactory> = Arc::new(MarkerFactory { kind: "tool" });
    let _f = factories.register("itest.module:build_tool", factory);

    // 2) 声明描述符(带构造输入 schema 的 tool)。
    let input_model = ConstructionInputModel {
        fields: vec![ah_contracts::manifest::InputFieldSpec::param_field(
            "tool_names",
            Some(serde_json::json!(["switch_mode"])),
            "Tool names from spec params.",
        )],
    };
    let descriptor = HarnessElementDescriptor {
        kind: ElementKind::Tool,
        name: "itest.web_search".to_string(),
        description: "Free web search tool.".to_string(),
        factory_ref: "itest.module:build_tool".to_string(),
        input_schema: input_model.json_schema(),
        input_model_ref: None,
        interface_methods: ah_contracts::manifest::default_interface_methods(ElementKind::Tool),
    };
    let _g = catalog.add_descriptor(descriptor).expect("add");

    // 3) 目录驱动注册(kind 路由)。
    let _reg = registration.register_from_catalog().expect("register all");

    // 4) 取回 provider 并真实调用,验证构造输入解析。
    let provider = registration
        .tool_provider("itest.web_search")
        .expect("provider");
    let context = MapManifestContext::from_pairs(vec![("language", Value::String("en".into()))]);
    let built = provider
        .build(
            &serde_json::json!({"tool_names": ["a"]}),
            &context,
            &*factories,
        )
        .expect("build");
    assert_eq!(built["kind"], "tool");
    assert_eq!(built["params"]["tool_names"][0], "a");

    // 5) 目录 JSON 导出可序列化往返。
    let elements = catalog.list_elements();
    let round = serde_json::to_string(&elements).expect("serialize");
    let back: Vec<Value> = serde_json::from_str(&round).expect("deserialize");
    assert_eq!(back.len(), 1);

    drop(effects);
}

#[test]
fn duplicate_name_rejected_and_effect_rolls_back() {
    let ctx = Context::new();
    let effects = mount(&ctx);
    let catalog = ctx
        .service::<dyn ManifestCatalog>(&MANIFEST)
        .expect("catalog");

    let d = HarnessElementDescriptor {
        kind: ElementKind::Rail,
        name: "itest.dup".to_string(),
        description: "first".to_string(),
        factory_ref: "mod:a".to_string(),
        input_schema: Value::Object(Default::default()),
        input_model_ref: None,
        interface_methods: Vec::new(),
    };
    let _g = catalog.add_descriptor(d).expect("add");
    let dup = HarnessElementDescriptor {
        kind: ElementKind::Rail,
        name: "itest.dup".to_string(),
        description: "second".to_string(),
        factory_ref: "mod:b".to_string(),
        input_schema: Value::Object(Default::default()),
        input_model_ref: None,
        interface_methods: Vec::new(),
    };
    let err = catalog.add_descriptor(dup).expect_err("duplicate rejected");
    assert!(err.0.contains("Duplicate harness element name"));

    // 具名 guard 持有期间存在;drop 后回滚。
    {
        let g = catalog.add_descriptor(HarnessElementDescriptor {
            kind: ElementKind::Tool,
            name: "itest.tmp".to_string(),
            description: "temporary".to_string(),
            factory_ref: "mod:c".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        });
        let _g = g.expect("add");
        assert!(catalog.get("itest.tmp").is_some());
    }
    assert!(catalog.get("itest.tmp").is_none());

    drop(effects);
}

#[test]
fn unresolved_factory_fails_explicitly() {
    let ctx = Context::new();
    let effects = mount(&ctx);
    let catalog = ctx
        .service::<dyn ManifestCatalog>(&MANIFEST)
        .expect("catalog");
    let registration = ctx
        .service::<dyn ManifestRegistration>(&MANIFEST_REGISTRATION)
        .expect("registration");
    let _g = catalog
        .add_descriptor(HarnessElementDescriptor {
            kind: ElementKind::Subagent,
            name: "itest.ghost".to_string(),
            description: String::new(),
            factory_ref: "mod:not_registered".to_string(),
            input_schema: Value::Object(Default::default()),
            input_model_ref: None,
            interface_methods: Vec::new(),
        })
        .expect("add");
    let err = registration
        .register_from_catalog()
        .expect_err("explicit error");
    assert!(err.0.contains("Cannot resolve factory"));
    drop(effects);
}
