//! P0-02:production profile 静态组合验证。
//!
//! 对 dev/prod profile 展开后的插件集(由 `ah-app::plugin_catalog` 解析)执行
//! **静态组合校验**(不调用 `apply()`,因此不依赖真实凭据/外部后端):
//!
//! 1. **可解析**:每个插件名都能在 catalog 中找到;
//! 2. **无重复 provider**:同一服务键至多一个插件 `provides`;
//! 3. **无缺失 provider**:每个插件的每个 `inject` 键都被组内某插件提供;
//! 4. **无环**:provides/inject 依赖图无环(挂载拓扑可确定)。
//!
//! 校验语义镜像 `ah-hub::Context::mount_all`(PluginError::DuplicateProvider /
//! MissingDependency / CycleDetected),但在不触发 `apply` 的前提下对真实 profile
//! 生效 —— 这正是 ROADMAP P0-02 要求、而 mount 路径无法静态给出的一层验证。
//! mock 门禁(prod 无 mock)也在此一并断言。

use std::collections::HashMap;

use ah_hub::plugin::DynPlugin;
use ah_hub::profile::Profile;

/// 组合校验发现的问题(全部列出,便于一次性修复)。
#[derive(Debug, PartialEq, Eq)]
enum Issue {
    UnknownPlugin(String),
    DuplicateProvider {
        key: &'static str,
        first: &'static str,
        second: &'static str,
    },
    MissingProvider {
        plugin: &'static str,
        key: &'static str,
    },
    Cycle {
        chain: Vec<&'static str>,
    },
}

impl Issue {
    fn render(&self) -> String {
        match self {
            Issue::UnknownPlugin(name) => format!("unknown plugin in catalog: {name}"),
            Issue::DuplicateProvider { key, first, second } => {
                format!("service `{key}` provided twice: `{first}` and `{second}`")
            }
            Issue::MissingProvider { plugin, key } => {
                format!("plugin `{plugin}` requires missing service `{key}`")
            }
            Issue::Cycle { chain } => format!("dependency cycle: {:?}", chain),
        }
    }
}

fn catalog() -> Vec<(&'static str, DynPlugin)> {
    let root = std::env::temp_dir().join(format!("ah-composition-{}", std::process::id()));
    let session_path = root.join("default.jsonl");
    let session_dir = root.join("sessions");
    ah_app::plugin_catalog(
        &root,
        &session_path,
        &session_dir,
        &root.join("memory"),
        &root.join("retrieval"),
        &root.join("telemetry"),
    )
}

fn profile_path(name: &str) -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/").to_string() + name
}

fn validate(profile_name: &str) -> Vec<Issue> {
    let profile =
        Profile::load(profile_path(profile_name)).unwrap_or_else(|e| panic!("{profile_name}: {e}"));
    let cat = catalog();
    let mut issues = Vec::new();

    // 1) 解析:每个插件名必须在 catalog 中。
    let names = profile.plugin_names();
    let mut set: Vec<&DynPlugin> = Vec::new();
    for name in &names {
        match cat.iter().find(|(n, _)| n == name).map(|(_, p)| p) {
            Some(p) => set.push(p),
            None => issues.push(Issue::UnknownPlugin(name.clone())),
        }
    }

    // 2) 无重复 provider。
    let mut providers: HashMap<&'static str, &'static str> = HashMap::new();
    for p in &set {
        for key in p.provides() {
            if let Some(first) = providers.insert(key.name(), p.name()) {
                issues.push(Issue::DuplicateProvider {
                    key: key.name(),
                    first,
                    second: p.name(),
                });
            }
        }
    }

    // 3) 无缺失 provider。
    for p in &set {
        for key in p.inject() {
            if !providers.contains_key(key.name()) {
                issues.push(Issue::MissingProvider {
                    plugin: p.name(),
                    key: key.name(),
                });
            }
        }
    }

    // 4) 无环:DFS(inject 键 -> 组内 provider 边),镜像 mount_all 的 visit。
    if providers.len() == set.len() {
        let index_of: HashMap<&'static str, usize> =
            set.iter().enumerate().map(|(i, p)| (p.name(), i)).collect();
        const WHITE: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;
        let mut state = vec![WHITE; set.len()];
        let mut path: Vec<&'static str> = Vec::new();
        let mut cycle: Option<Vec<&'static str>> = None;

        fn visit(
            i: usize,
            set: &[&DynPlugin],
            providers: &HashMap<&'static str, &'static str>,
            index_of: &HashMap<&'static str, usize>,
            state: &mut [u8],
            path: &mut Vec<&'static str>,
        ) -> Option<Vec<&'static str>> {
            match state[i] {
                BLACK => return None,
                GRAY => {
                    path.push(set[i].name());
                    return Some(path.clone());
                }
                _ => {}
            }
            state[i] = GRAY;
            path.push(set[i].name());
            for key in set[i].inject() {
                if let Some(&provider) = providers.get(key.name())
                    && let Some(&dep) = index_of.get(provider)
                    && let Some(chain) = visit(dep, set, providers, index_of, state, path)
                {
                    return Some(chain);
                }
            }
            path.pop();
            state[i] = BLACK;
            None
        }

        for i in 0..set.len() {
            if let Some(chain) = visit(i, &set, &providers, &index_of, &mut state, &mut path) {
                cycle = Some(chain);
                break;
            }
        }
        if let Some(chain) = cycle {
            issues.push(Issue::Cycle { chain });
        }
    }

    issues
}

#[test]
fn prod_profile_static_composition_is_valid() {
    let issues = validate("prod.toml");
    assert!(
        issues.is_empty(),
        "prod profile composition issues:\n{}",
        issues
            .iter()
            .map(Issue::render)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn dev_profile_static_composition_is_valid() {
    let issues = validate("dev.toml");
    assert!(
        issues.is_empty(),
        "dev profile composition issues:\n{}",
        issues
            .iter()
            .map(Issue::render)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn prod_profile_has_no_mock() {
    let profile = Profile::load(profile_path("prod.toml")).expect("prod profile");
    assert!(
        !profile
            .plugin_names()
            .iter()
            .any(|p| p == "ah-plugins-mock"),
        "prod profile must not contain mock plugins"
    );
}

#[test]
fn catalog_resolves_every_declared_plugin() {
    let cat = catalog();
    for profile_name in ["dev.toml", "prod.toml"] {
        let profile = Profile::load(profile_path(profile_name)).unwrap();
        for name in profile.plugin_names() {
            assert!(
                cat.iter().any(|(n, _)| n == &name),
                "{profile_name}: plugin `{name}` not resolvable in catalog"
            );
        }
    }
}
