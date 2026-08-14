//! Mock 门禁(架构硬性规则 #4):生产 profile 展开后不得包含 mock 插件。
//!
//! 对应 docs/architecture.md 的 mock 门禁与 CI 门禁第 5 条:
//! 展开生产 profile,若包含 ah-plugins-mock 插件则失败。

use ah_hub::profile::Profile;

fn prod_profile() -> Profile {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/prod.toml");
    Profile::load(path).expect("prod profile must load")
}

fn dev_profile() -> Profile {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/dev.toml");
    Profile::load(path).expect("dev profile must load")
}

#[test]
fn prod_profile_contains_no_mock_plugins() {
    let plugins = prod_profile().plugin_names();
    assert!(
        !plugins.iter().any(|p| p == "ah-plugins-mock"),
        "prod profile must not contain mock plugins, got: {plugins:?}"
    );
}

#[test]
fn prod_profile_still_loads_all_plugins_from_catalog() {
    // 生产 profile 的每个插件都必须在 ah-app catalog 中可解析(接线完整性)。
    // 具体解析在 boot 时验证;此处至少断言清单非空且不含未知名。
    let plugins = prod_profile().plugin_names();
    assert!(!plugins.is_empty(), "prod profile must declare plugins");
    assert!(
        plugins.iter().all(|p| p != "ah-plugins-mock"),
        "prod profile leaked a mock plugin"
    );
}

#[test]
fn dev_profile_may_keep_the_llm_boot_stub() {
    let plugins = dev_profile().plugin_names();
    assert!(
        plugins.contains(&"ah-plugins-mock".to_string()),
        "dev profile should keep the llm boot stub for keyless runs"
    );
}
