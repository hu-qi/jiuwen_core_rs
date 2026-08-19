//! 演进归档对(对齐 experience/archive.py 的纯逻辑部分)。
//!
//! 纯逻辑:版本规范化(latest/SKILL. 前缀剥除/v 前缀校验)+ 归档文件命名约定 +
//! 完整配对列表(新→旧)+ 修剪计数 + 下一版本号(冲突后缀)。

use serde_json::{Value, json};
use std::collections::BTreeSet;

pub const EVOLUTION_FILENAME: &str = "evolutions.json";
pub const LATEST_VERSION: &str = "latest";
pub const SKILL_ARCHIVE_PREFIX: &str = "SKILL.";
pub const SKILL_ARCHIVE_SUFFIX: &str = ".md";
pub const EVOLUTION_ARCHIVE_PREFIX: &str = "evolutions.";
pub const EVOLUTION_ARCHIVE_SUFFIX: &str = ".json";
pub const DEFAULT_ARCHIVE_KEEP_LATEST: usize = 10;

/// 可恢复的 SKILL.md + evolutions.json 归档对(对齐 EvolutionArchivePair)。
#[derive(Debug, Clone, PartialEq)]
pub struct EvolutionArchivePair {
    pub version: String,
    pub skill_archive: String,
    pub evolution_archive: String,
}

impl EvolutionArchivePair {
    pub fn skill_archive_name(&self) -> &str {
        &self.skill_archive
    }

    pub fn evolution_archive_name(&self) -> &str {
        &self.evolution_archive
    }

    pub fn to_payload(&self) -> Value {
        json!({
            "version": self.version,
            "skill_archive": self.skill_archive_name(),
            "evolution_archive": self.evolution_archive_name(),
        })
    }
}

/// 规范化用户面版本令牌(对齐 normalize_version)。
pub fn normalize_version(raw: &str) -> Option<String> {
    let version = raw.trim();
    if version.is_empty() {
        return None;
    }
    if version == LATEST_VERSION {
        return Some(LATEST_VERSION.to_string());
    }
    let mut version = version.to_string();
    if version.starts_with(SKILL_ARCHIVE_PREFIX) && version.ends_with(SKILL_ARCHIVE_SUFFIX) {
        version = version[SKILL_ARCHIVE_PREFIX.len()..version.len() - SKILL_ARCHIVE_SUFFIX.len()]
            .to_string();
    }
    if version.starts_with('v') {
        Some(version)
    } else {
        None
    }
}

/// SKILL 归档文件名(对齐 _skill_archive_name)。
pub fn skill_archive_name(version: &str) -> String {
    format!("{SKILL_ARCHIVE_PREFIX}{version}{SKILL_ARCHIVE_SUFFIX}")
}

/// evolutions 归档文件名(对齐 _evolution_archive_name)。
pub fn evolution_archive_name(version: &str) -> String {
    format!("{EVOLUTION_ARCHIVE_PREFIX}{version}{EVOLUTION_ARCHIVE_SUFFIX}")
}

/// 从 SKILL 归档文件名提取版本(对齐 _version_from_skill_archive_name)。
pub fn version_from_skill_archive_name(filename: &str) -> Option<String> {
    if !filename.starts_with(SKILL_ARCHIVE_PREFIX) || !filename.ends_with(SKILL_ARCHIVE_SUFFIX) {
        return None;
    }
    let version = filename[SKILL_ARCHIVE_PREFIX.len()..filename.len() - SKILL_ARCHIVE_SUFFIX.len()]
        .to_string();
    if version.starts_with('v') {
        Some(version)
    } else {
        None
    }
}

/// 完整归档对列表(新→旧;对齐 list_pairs 的配对逻辑)。
pub fn list_pairs(archive_names: &BTreeSet<String>) -> Vec<EvolutionArchivePair> {
    let mut names: Vec<&String> = archive_names.iter().collect();
    names.sort();
    names.reverse();
    let mut pairs = Vec::new();
    for filename in names {
        let Some(version) = version_from_skill_archive_name(filename) else {
            continue;
        };
        let evo_name = evolution_archive_name(&version);
        if !archive_names.contains(&evo_name) {
            continue;
        }
        pairs.push(EvolutionArchivePair {
            version,
            skill_archive: filename.clone(),
            evolution_archive: evo_name,
        });
    }
    pairs
}

/// 修剪计数(对齐 prune 的计数逻辑;返回应移除的对数)。
pub fn prune_count(pairs: &[EvolutionArchivePair], keep_latest: Option<usize>) -> usize {
    let keep = keep_latest.unwrap_or(DEFAULT_ARCHIVE_KEEP_LATEST);
    let keep = keep.max(0);
    pairs.len().saturating_sub(keep)
}

/// 下一版本号(对齐 _next_pair_version:基础时间戳 + 冲突后缀)。
///
/// exists 判定归档文件是否已存在(真实 fs 检查留待集成)。
pub fn next_pair_version(base_version: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    let mut version = base_version.to_string();
    let mut suffix = 1usize;
    while exists(&skill_archive_name(&version)) || exists(&evolution_archive_name(&version)) {
        suffix += 1;
        version = format!("{base_version}_{suffix:02}");
    }
    version
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_version_rules() {
        assert_eq!(normalize_version("latest").as_deref(), Some("latest"));
        assert_eq!(normalize_version("v1.2.3").as_deref(), Some("v1.2.3"));
        assert_eq!(
            normalize_version("SKILL.v20260101T120000.md").as_deref(),
            Some("v20260101T120000"),
        );
        assert_eq!(normalize_version("  "), None);
        assert_eq!(normalize_version("1.2.3"), None);
        assert_eq!(normalize_version(""), None);
    }

    #[test]
    fn archive_name_conventions() {
        assert_eq!(skill_archive_name("v1"), "SKILL.v1.md");
        assert_eq!(evolution_archive_name("v1"), "evolutions.v1.json");
        assert_eq!(
            version_from_skill_archive_name("SKILL.v1.md").as_deref(),
            Some("v1")
        );
        assert_eq!(version_from_skill_archive_name("SKILL.latest.md"), None);
        assert_eq!(version_from_skill_archive_name("other.md"), None);
    }

    #[test]
    fn pair_listing_newest_first_and_complete_only() {
        let names = BTreeSet::from([
            "SKILL.v1.md".to_string(),
            "evolutions.v1.json".to_string(),
            "SKILL.v2.md".to_string(), // 缺 evolutions.v2.json → 不成对
            "SKILL.latest.md".to_string(),
            "evolutions.latest.json".to_string(),
            "SKILL.v0.md".to_string(),
            "evolutions.v0.json".to_string(),
            "unrelated.txt".to_string(),
        ]);
        let pairs = list_pairs(&names);
        assert_eq!(pairs.len(), 2);
        // 字典序逆序:v2/latest 不成对或非 v 前缀被跳过,v1 > v0
        assert_eq!(pairs[0].version, "v1");
        assert_eq!(pairs[1].version, "v0");
        assert_eq!(pairs[0].skill_archive, "SKILL.v1.md");
        assert_eq!(pairs[0].evolution_archive, "evolutions.v1.json");
    }

    #[test]
    fn pair_payload() {
        let pair = EvolutionArchivePair {
            version: "v1".to_string(),
            skill_archive: "SKILL.v1.md".to_string(),
            evolution_archive: "evolutions.v1.json".to_string(),
        };
        let payload = pair.to_payload();
        assert_eq!(payload["version"], "v1");
        assert_eq!(payload["skill_archive"], "SKILL.v1.md");
        assert_eq!(payload["evolution_archive"], "evolutions.v1.json");
        assert!(json!(payload).is_object());
    }

    #[test]
    fn prune_count_logic() {
        let pairs: Vec<EvolutionArchivePair> = (0..5)
            .map(|i| EvolutionArchivePair {
                version: format!("v{i}"),
                skill_archive: format!("SKILL.v{i}.md"),
                evolution_archive: format!("evolutions.v{i}.json"),
            })
            .collect();
        assert_eq!(prune_count(&pairs, Some(3)), 2);
        assert_eq!(prune_count(&pairs, Some(10)), 0);
        assert_eq!(prune_count(&pairs, None), 0);
        assert_eq!(prune_count(&pairs, Some(0)), 5);
    }

    #[test]
    fn next_version_suffix_loop() {
        let existing = BTreeSet::from([
            "SKILL.v1.md".to_string(),
            "evolutions.v1_02.json".to_string(),
        ]);
        let v = next_pair_version("v1", |name| existing.contains(name));
        assert_eq!(v, "v1_03");
        let v2 = next_pair_version("v2", |name| existing.contains(name));
        assert_eq!(v2, "v2");
    }
}
