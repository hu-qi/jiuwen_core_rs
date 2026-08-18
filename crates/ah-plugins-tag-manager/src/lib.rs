//! # ah-plugins-tag-manager
//!
//! Real resource tag manager (aligned with
//! core/runner/resources_manager/tag_manager.py):
//! - bidirectional index: resource -> tags, tag -> resources;
//! - GLOBAL semantics: global-tagged resources cannot carry other tags;
//! - TagUpdateStrategy (replace/merge) and TagMatchStrategy (any/all);
//! - full CRUD: tag/remove/update/remove-tag/find/has.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ah_contracts::keys::TAG_MANAGER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tag_manager::{GLOBAL, Tag, TagError, TagMatchStrategy, TagUpdateStrategy};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 资源标签管理器(双向索引;对齐 TagMgr)。
pub struct TagMgr {
    /// resource_id -> tags。
    resource_tags: HashMap<String, HashSet<Tag>>,
    /// tag -> resource_ids。
    tag_to_resource: HashMap<Tag, HashSet<String>>,
}

impl TagMgr {
    pub fn new() -> Self {
        let mut tag_to_resource = HashMap::new();
        tag_to_resource.insert(GLOBAL.to_string(), HashSet::new());
        Self {
            resource_tags: HashMap::new(),
            tag_to_resource,
        }
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tag_to_resource.contains_key(tag)
    }

    /// 全部非空标签(对齐 list_tags)。
    pub fn list_tags(&self) -> Vec<Tag> {
        self.tag_to_resource
            .iter()
            .filter(|(_, r)| !r.is_empty())
            .map(|(t, _)| t.clone())
            .collect()
    }

    pub fn has_resource(&self, resource_id: &str) -> bool {
        self.resource_tags.contains_key(resource_id)
    }

    /// 给资源打标签(含 GLOBAL;原子操作;对齐 tag_resource)。
    pub fn tag_resource(&mut self, resource_id: &str, tags: &[String]) -> Vec<Tag> {
        let tags_to_add = normalize_tags(tags);
        self.resource_tags
            .entry(resource_id.to_string())
            .or_default();
        if tags_to_add.contains(GLOBAL) {
            return self.set_global_resource(resource_id);
        }
        self.add_resource_tags(resource_id, &tags_to_add)
    }

    /// 完全移除资源及其全部标签(对齐 remove_resource)。
    pub fn remove_resource(&mut self, resource_id: &str) -> Vec<Tag> {
        if !self.resource_tags.contains_key(resource_id) {
            return Vec::new();
        }
        self.remove_resource_inner(resource_id)
    }

    /// 从资源移除指定标签(对齐 remove_resource_tags)。
    pub fn remove_resource_tags(
        &mut self,
        resource_id: &str,
        tags: &[String],
        skip_if_not_exists: bool,
    ) -> Result<Vec<Tag>, TagError> {
        let tags_to_remove = normalize_tags(tags);
        let current_tags = self.resource_tags.get(resource_id).ok_or_else(|| {
            TagError::new(
                "resource_tag_remove_error",
                format!("Resource does not exist: {resource_id}"),
            )
        })?;
        if !skip_if_not_exists {
            let missing: Vec<String> = tags_to_remove.difference(current_tags).cloned().collect();
            if !missing.is_empty() {
                return Err(TagError::new(
                    "resource_tag_remove_error",
                    format!("Tag does not exist: {:?}", missing),
                ));
            }
        }
        Ok(self.remove_resource_tags_inner(resource_id, &tags_to_remove))
    }

    /// 按策略更新资源标签(对齐 update_resource_tags)。
    pub fn update_resource_tags(
        &mut self,
        resource_id: &str,
        tags: &[String],
        strategy: TagUpdateStrategy,
    ) -> Result<Vec<Tag>, TagError> {
        let new_tags = normalize_tags(tags);
        if !self.resource_tags.contains_key(resource_id) {
            return Err(TagError::new(
                "resource_tag_replace_error",
                format!("Resource does not exist: {resource_id}"),
            ));
        }
        if new_tags.contains(GLOBAL) {
            return Ok(self.set_global_resource(resource_id));
        }
        match strategy {
            TagUpdateStrategy::Replace => Ok(self.replace_resource_tags(resource_id, &new_tags)),
            TagUpdateStrategy::Merge => Ok(self.add_resource_tags(resource_id, &new_tags)),
        }
    }

    /// 完全移除标签及其全部关联(对齐 remove_tag)。
    pub fn remove_tag(
        &mut self,
        tag: &str,
        skip_if_not_exists: bool,
    ) -> Result<Vec<String>, TagError> {
        if !self.tag_to_resource.contains_key(tag) {
            if skip_if_not_exists {
                return Ok(Vec::new());
            }
            return Err(TagError::new(
                "resource_tag_remove_tag_error",
                format!("Tag does not exist: {tag}"),
            ));
        }
        Ok(self.remove_tag_inner(tag))
    }

    /// 指定标签的资源列表(对齐 get_tag_resources)。
    pub fn get_tag_resources(&self, tag: &str) -> Vec<String> {
        self.tag_to_resource
            .get(tag)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// 按策略查找资源(对齐 find_resources_by_tags)。
    pub fn find_resources_by_tags(
        &self,
        tags: &[String],
        strategy: TagMatchStrategy,
        skip_if_not_exists: bool,
    ) -> Result<Vec<String>, TagError> {
        let tags_to_search = normalize_tags(tags);
        match strategy {
            TagMatchStrategy::Any => {
                let mut found = HashSet::new();
                for tag in &tags_to_search {
                    let resources = self.tag_to_resource.get(tag);
                    match resources {
                        Some(r) => {
                            found.extend(r.iter().cloned());
                        }
                        None => {
                            if !is_builtin_tag(tag) && !skip_if_not_exists {
                                return Err(TagError::new(
                                    "resource_tag_find_error",
                                    format!("Tag does not exist: {tag}"),
                                ));
                            }
                        }
                    }
                }
                Ok(found.into_iter().collect())
            }
            TagMatchStrategy::All => {
                self.find_resources_with_all_tags(&tags_to_search, skip_if_not_exists)
            }
        }
    }

    /// 资源是否含指定标签(对齐 has_resource_tag)。
    pub fn has_resource_tag(&self, resource_id: &str, tag: &str) -> bool {
        self.resource_tags
            .get(resource_id)
            .map(|t| t.contains(tag))
            .unwrap_or(false)
    }

    /// 资源的全部标签(对齐 get_resources_tags)。
    pub fn get_resources_tags(&self, resource_id: &str) -> Vec<Tag> {
        self.resource_tags
            .get(resource_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// 当前状态统计(对齐 display 的统计部分)。
    pub fn stats(&self) -> (usize, usize, usize) {
        let total_tags = self.tag_to_resource.len();
        let total_resources = self.resource_tags.len();
        let global_resources = self
            .tag_to_resource
            .get(GLOBAL)
            .map(|s| s.len())
            .unwrap_or(0);
        (total_tags, total_resources, global_resources)
    }

    // --- private helpers (aligned with _-prefixed Python methods) ---

    fn set_global_resource(&mut self, resource_id: &str) -> Vec<Tag> {
        let old_tags: Vec<Tag> = self
            .resource_tags
            .get(resource_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for old_tag in &old_tags {
            if let Some(set) = self.tag_to_resource.get_mut(old_tag) {
                set.remove(resource_id);
                if set.is_empty() && old_tag != GLOBAL {
                    self.tag_to_resource.remove(old_tag);
                }
            }
        }
        self.resource_tags.insert(resource_id.to_string(), {
            let mut s = HashSet::new();
            s.insert(GLOBAL.to_string());
            s
        });
        self.tag_to_resource
            .entry(GLOBAL.to_string())
            .or_default()
            .insert(resource_id.to_string());
        old_tags
    }

    fn add_resource_tags(&mut self, resource_id: &str, tags_to_add: &HashSet<Tag>) -> Vec<Tag> {
        if !self.resource_tags.contains_key(resource_id) {
            return Vec::new();
        }
        let current = self.resource_tags.get_mut(resource_id).unwrap();
        if current.contains(GLOBAL) {
            return vec![GLOBAL.to_string()];
        }
        current.extend(tags_to_add.iter().cloned());
        for tag in tags_to_add {
            self.tag_to_resource
                .entry(tag.clone())
                .or_default()
                .insert(resource_id.to_string());
        }
        current.iter().cloned().collect()
    }

    fn remove_resource_inner(&mut self, resource_id: &str) -> Vec<Tag> {
        let tags: Vec<Tag> = self
            .resource_tags
            .remove(resource_id)
            .unwrap_or_default()
            .into_iter()
            .collect();
        for tag in &tags {
            if let Some(set) = self.tag_to_resource.get_mut(tag) {
                set.remove(resource_id);
                if set.is_empty() && tag != GLOBAL {
                    self.tag_to_resource.remove(tag);
                }
            }
        }
        tags
    }

    fn remove_resource_tags_inner(
        &mut self,
        resource_id: &str,
        tags_to_remove: &HashSet<Tag>,
    ) -> Vec<Tag> {
        if !self.resource_tags.contains_key(resource_id) {
            return Vec::new();
        }
        let removed: Vec<Tag> = self
            .resource_tags
            .get_mut(resource_id)
            .unwrap()
            .iter()
            .filter(|t| tags_to_remove.contains(*t))
            .cloned()
            .collect();
        let empty_now = {
            let current = self.resource_tags.get_mut(resource_id).unwrap();
            for tag in &removed {
                current.remove(tag);
                if let Some(set) = self.tag_to_resource.get_mut(tag) {
                    set.remove(resource_id);
                    if set.is_empty() && tag != GLOBAL {
                        self.tag_to_resource.remove(tag);
                    }
                }
            }
            current.is_empty()
        };
        if empty_now {
            self.resource_tags.remove(resource_id);
        }
        self.resource_tags
            .get(resource_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    fn replace_resource_tags(&mut self, resource_id: &str, new_tags: &HashSet<Tag>) -> Vec<Tag> {
        if !self.resource_tags.contains_key(resource_id) {
            return Vec::new();
        }
        let old_tags: Vec<Tag> = self
            .resource_tags
            .get(resource_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for old_tag in &old_tags {
            if let Some(set) = self.tag_to_resource.get_mut(old_tag) {
                set.remove(resource_id);
                if set.is_empty() && old_tag != GLOBAL {
                    self.tag_to_resource.remove(old_tag);
                }
            }
        }
        self.resource_tags
            .insert(resource_id.to_string(), new_tags.clone());
        for tag in new_tags {
            self.tag_to_resource
                .entry(tag.clone())
                .or_default()
                .insert(resource_id.to_string());
        }
        new_tags.iter().cloned().collect()
    }

    fn remove_tag_inner(&mut self, tag: &str) -> Vec<String> {
        let affected: Vec<String> = self
            .tag_to_resource
            .get(tag)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for resource_id in &affected {
            if let Some(tags) = self.resource_tags.get_mut(resource_id) {
                tags.remove(tag);
                if tags.is_empty() {
                    self.resource_tags.remove(resource_id);
                }
            }
        }
        self.tag_to_resource.remove(tag);
        affected
    }

    fn find_resources_with_all_tags(
        &self,
        required_tags: &HashSet<Tag>,
        skip_if_not_exists: bool,
    ) -> Result<Vec<String>, TagError> {
        if required_tags.is_empty() {
            return Ok(Vec::new());
        }
        for tag in required_tags {
            if !self.tag_to_resource.contains_key(tag)
                && !is_builtin_tag(tag)
                && !skip_if_not_exists
            {
                return Err(TagError::new(
                    "resource_tag_find_error",
                    format!("Tag does not exist: {tag}"),
                ));
            }
        }
        let first = required_tags.iter().next().unwrap();
        let mut found: HashSet<String> =
            self.tag_to_resource.get(first).cloned().unwrap_or_default();
        for tag in required_tags {
            let resources = self.tag_to_resource.get(tag).cloned().unwrap_or_default();
            found.retain(|r| resources.contains(r));
        }
        Ok(found.into_iter().collect())
    }
}

impl Default for TagMgr {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for TagMgr {}

fn normalize_tags(tags: &[String]) -> HashSet<Tag> {
    tags.iter().cloned().collect()
}

fn is_builtin_tag(tag: &str) -> bool {
    tag == GLOBAL
}

/// tag-manager 插件:注册 TagMgr。
pub struct TagManagerPlugin;

impl Plugin for TagManagerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tag-manager"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TAG_MANAGER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let mgr = Arc::new(TagMgr::new());
        Ok(vec![ctx.register(TAG_MANAGER, mgr)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn tag_and_query_roundtrip() {
        let mut m = TagMgr::new();
        m.tag_resource("agent1", &[tag("coder"), tag("fast")]);
        assert!(m.has_resource("agent1"));
        assert!(m.has_resource_tag("agent1", "coder"));
        let tags = m.get_resources_tags("agent1");
        assert_eq!(tags.len(), 2);
        let res = m
            .find_resources_by_tags(&[tag("coder")], TagMatchStrategy::Any, true)
            .unwrap();
        assert_eq!(res, vec!["agent1".to_string()]);
    }

    #[test]
    fn global_tag_replaces_others() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x")]);
        let old = m.tag_resource("a", &[tag(GLOBAL)]);
        assert_eq!(old, vec![tag("x")]);
        assert_eq!(m.get_resources_tags("a"), vec![tag(GLOBAL)]);
        assert!(!m.has_resource_tag("a", "x"));
        let globals = m
            .find_resources_by_tags(&[tag(GLOBAL)], TagMatchStrategy::Any, true)
            .unwrap();
        assert!(globals.contains(&"a".to_string()));
    }

    #[test]
    fn global_resource_rejects_other_tags() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag(GLOBAL)]);
        let r = m.add_resource_tags("a", &normalize_tags(&[tag("x")]));
        assert_eq!(r, vec![tag(GLOBAL)]);
        assert!(!m.has_resource_tag("a", "x"));
    }

    #[test]
    fn update_replace_and_merge() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x")]);
        m.update_resource_tags("a", &[tag("y")], TagUpdateStrategy::Merge)
            .unwrap();
        assert!(m.has_resource_tag("a", "x") && m.has_resource_tag("a", "y"));
        m.update_resource_tags("a", &[tag("z")], TagUpdateStrategy::Replace)
            .unwrap();
        assert!(!m.has_resource_tag("a", "x"));
        assert!(m.has_resource_tag("a", "z"));
    }

    #[test]
    fn remove_resource_and_tags() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x"), tag("y")]);
        let remaining = m.remove_resource_tags("a", &[tag("x")], false).unwrap();
        assert_eq!(remaining, vec![tag("y")]);
        m.remove_resource("a");
        assert!(!m.has_resource("a"));
        assert_eq!(m.get_tag_resources("x").len(), 0);
    }

    #[test]
    fn find_all_requires_every_tag() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x"), tag("y")]);
        m.tag_resource("b", &[tag("x")]);
        let both = m
            .find_resources_by_tags(&[tag("x"), tag("y")], TagMatchStrategy::All, true)
            .unwrap();
        assert_eq!(both, vec!["a".to_string()]);
        let any = m
            .find_resources_by_tags(&[tag("x"), tag("z")], TagMatchStrategy::Any, true)
            .unwrap();
        assert!(any.contains(&"a".to_string()) && any.contains(&"b".to_string()));
    }

    #[test]
    fn remove_tag_affects_all_resources() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x")]);
        m.tag_resource("b", &[tag("x")]);
        let affected = m.remove_tag("x", false).unwrap();
        assert_eq!(affected.len(), 2);
        assert!(!m.has_tag("x"));
        assert!(!m.has_resource_tag("a", "x"));
    }

    #[test]
    fn stats_and_list() {
        let mut m = TagMgr::new();
        assert_eq!(m.stats().1, 0);
        m.tag_resource("a", &[tag("x")]);
        assert_eq!(m.stats().1, 1);
        assert!(m.list_tags().contains(&tag("x")));
    }

    #[test]
    fn missing_tag_errors_without_skip() {
        let mut m = TagMgr::new();
        m.tag_resource("a", &[tag("x")]);
        let err = m
            .remove_resource_tags("a", &[tag("missing")], false)
            .unwrap_err();
        assert!(err.message.contains("does not exist"));
        let err2 = m.remove_tag("nope", false).unwrap_err();
        assert!(err2.message.contains("does not exist"));
    }
}
