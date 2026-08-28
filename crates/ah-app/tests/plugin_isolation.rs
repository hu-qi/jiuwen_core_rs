//! P1-08:插件依赖隔离 CI 门禁。
//!
//! 硬性规则(见 docs/architecture.md §4.2 与 docs/ROADMAP.md P1-08):
//! - 除 `ah-app`(boot 组装层)外,任何 crate 的**生产** `[dependencies]` 禁止引用
//!   其他 `ah-plugins-*` crate(插件间不得互相依赖具体类型);
//! - dev-dependencies 允许(测试需要组合多个插件);
//! - 附带规则:插件 crate(`ah-plugins-*`)的生产依赖中,内部 crate 只允许
//!   `ah-hub` / `ah-contracts`(禁止依赖 `ah-app` 或其他插件)。
//!
//! 当前无违规;本文件把规则固化为 CI 门禁(`cargo test --workspace` 自动覆盖),
//! 防止后续改动回归。

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// 按节(section)解析 Cargo.toml,仅收集 `ah-*` 内部依赖名。
fn sections(path: &Path) -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(sec) = current.take() {
                out.push(sec);
            }
            current = Some((
                line.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string(),
                Vec::new(),
            ));
            continue;
        }
        if let Some((_, deps)) = current.as_mut()
            && let Some((name, _)) = line.split_once('=')
        {
            let name = name.trim();
            let is_ah = name.starts_with("ah-");
            let well_formed = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if is_ah && well_formed {
                deps.push(name.to_string());
            }
        }
    }
    if let Some(sec) = current {
        out.push(sec);
    }
    out
}

fn crate_dirs() -> Vec<PathBuf> {
    let crates = repo_root().join("crates");
    std::fs::read_dir(&crates)
        .expect("crates dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").exists())
        .collect()
}

fn prod_deps(path: &Path) -> Vec<String> {
    sections(path)
        .into_iter()
        .find(|(name, _)| name == "dependencies")
        .map(|(_, deps)| deps)
        .unwrap_or_default()
}

#[test]
fn no_plugin_in_production_deps_except_app() {
    let mut offenders = Vec::new();
    for dir in crate_dirs() {
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        if name == "ah-app" {
            continue;
        }
        let deps = prod_deps(&dir.join("Cargo.toml"));
        let plugins: Vec<&String> = deps
            .iter()
            .filter(|d| d.starts_with("ah-plugins-"))
            .collect();
        if !plugins.is_empty() {
            offenders.push(format!("{name}: {plugins:?}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "production [dependencies] of non-app crates must not reference ah-plugins-*:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn plugins_only_depend_on_hub_and_contracts() {
    let mut offenders = Vec::new();
    for dir in crate_dirs() {
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        if !name.starts_with("ah-plugins-") {
            continue;
        }
        let deps = prod_deps(&dir.join("Cargo.toml"));
        let internal: Vec<&String> = deps
            .iter()
            .filter(|d| d.starts_with("ah-"))
            .filter(|d| !matches!(d.as_str(), "ah-hub" | "ah-contracts"))
            .collect();
        if !internal.is_empty() {
            offenders.push(format!("{name}: {internal:?}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "plugin crates may only depend on ah-hub/ah-contracts internally:\n{}",
        offenders.join("\n")
    );
}
