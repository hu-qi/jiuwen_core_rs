//! # ah-plugins-graph-memory
//!
//! 真实知识图谱记忆(对齐 Python memory/graph):
//! - 实体抽取:字母数字词 + CJK 二元组(确定性,无 LLM 依赖);
//! - 关系:同片段共现实体对生成 relation;
//! - episode:每次 add_memory 一条,记录提及实体;
//! - 合并去重:同名实体按规范化名归并;
//! - JSONL 持久化(entities/relations/episodes 三集合),启动恢复;
//! - 检索:关键词命中统一打分(实体/关系/episode);邻居遍历。
//!
//! 提供 graph_add_memory / graph_search / graph_neighbors 三个真实工具。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::graph_memory::{
    AddMemoryResult, Entity, Episode, GraphHit, GraphMemory, GraphMemoryError, Relation,
};
use ah_contracts::keys::{GRAPH_MEMORY, TOOLS};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn uuid(tag: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    format!("{tag}-{}-{}-{seq}", now_ms(), std::process::id())
}

/// 规范化实体名(小写 + 空白折叠)。
fn normalize(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 确定性实体抽取:字母数字词 + CJK 二元组。
/// 返回 (实体名, 出现次数)。
fn extract_entities(text: &str) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    // 字母数字词。
    while i < chars.len() {
        if chars[i].is_alphanumeric() {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word.chars().count() >= 2 && !word.chars().all(|c| c.is_ascii_digit()) {
                *counts.entry(normalize(&word)).or_insert(0) += 1;
            }
        } else {
            i += 1;
        }
    }
    // CJK 二元组。
    let cjk: Vec<(usize, char)> = chars
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if ('\u{4e00}'..='\u{9fff}').contains(c) {
                Some((i, *c))
            } else {
                None
            }
        })
        .collect();
    for pair in cjk.windows(2) {
        let (i0, c0) = pair[0];
        let (i1, c1) = pair[1];
        if i1 == i0 + 1 {
            let mut s = String::with_capacity(4);
            s.push(c0);
            s.push(c1);
            *counts.entry(s).or_insert(0) += 1;
        }
    }
    // 过滤只出现一次的候选(噪声)。
    let mut result: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1 || text.len() <= 60)
        .collect();
    result.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    result
}

/// 真实 JSONL 图谱记忆。
pub struct JsonlGraphMemory {
    dir: PathBuf,
    entities: Mutex<HashMap<String, Entity>>, // uuid -> entity
    relations: Mutex<Vec<Relation>>,
    episodes: Mutex<Vec<Episode>>,
}

impl JsonlGraphMemory {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, GraphMemoryError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| GraphMemoryError(format!("create graph dir failed: {e}")))?;
        let mut entities = HashMap::new();
        let mut relations = Vec::new();
        let mut episodes = Vec::new();
        Self::load(&dir.join("entities.jsonl"), &mut |line| {
            if let Ok(entity) = serde_json::from_str::<Entity>(line) {
                entities.insert(entity.uuid.clone(), entity);
            }
        });
        Self::load(&dir.join("relations.jsonl"), &mut |line| {
            if let Ok(relation) = serde_json::from_str::<Relation>(line) {
                relations.push(relation);
            }
        });
        Self::load(&dir.join("episodes.jsonl"), &mut |line| {
            if let Ok(episode) = serde_json::from_str::<Episode>(line) {
                episodes.push(episode);
            }
        });
        Ok(Self {
            dir,
            entities: Mutex::new(entities),
            relations: Mutex::new(relations),
            episodes: Mutex::new(episodes),
        })
    }

    fn load(path: &Path, f: &mut dyn FnMut(&str)) {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if !line.trim().is_empty() {
                    f(line);
                }
            }
        }
    }

    fn append(path: &Path, line: &str) -> Result<(), GraphMemoryError> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| GraphMemoryError(format!("open graph file: {e}")))?;
        writeln!(file, "{line}").map_err(|e| GraphMemoryError(format!("append graph: {e}")))?;
        Ok(())
    }

    /// 按规范化名取实体。
    fn entity_by_normalized(&self, name: &str) -> Option<Entity> {
        let target = normalize(name);
        self.entities
            .lock()
            .unwrap()
            .values()
            .find(|e| normalize(&e.name) == target)
            .cloned()
    }
}

impl Seam for JsonlGraphMemory {}

impl GraphMemory for JsonlGraphMemory {
    fn add_memory(&self, content: &str) -> Result<AddMemoryResult, GraphMemoryError> {
        if content.trim().is_empty() {
            return Err(GraphMemoryError("content must not be empty".to_string()));
        }
        // 1) 抽取实体。
        let extracted = extract_entities(content);
        let mut added_entities = Vec::new();
        let mut mentioned: Vec<String> = Vec::new(); // entity uuids
        {
            let mut entities = self.entities.lock().unwrap();
            for (name, _) in extracted {
                if let Some(existing) = entities
                    .values()
                    .find(|e| normalize(&e.name) == normalize(&name))
                    .cloned()
                {
                    mentioned.push(existing.uuid);
                } else {
                    let entity = Entity {
                        uuid: uuid("ent"),
                        name,
                        content: content.chars().take(200).collect(),
                        relations: vec![],
                        episodes: vec![],
                        attributes: json!({}),
                    };
                    added_entities.push(entity.name.clone());
                    mentioned.push(entity.uuid.clone());
                    entities.insert(entity.uuid.clone(), entity);
                }
            }
            // 持久化新增实体(全量重写简单可靠)。
        }
        // 持久化:重写 entities.jsonl。
        {
            let entities = self.entities.lock().unwrap();
            let mut lines: Vec<String> = entities
                .values()
                .map(|e| serde_json::to_string(e).expect("serialize entity"))
                .collect();
            lines.sort();
            let path = self.dir.join("entities.jsonl");
            std::fs::write(&path, lines.join("\n"))
                .map_err(|e| GraphMemoryError(format!("write entities: {e}")))?;
        }

        // 2) 共现实体对 → relation。
        let mut added_relations = Vec::new();
        if mentioned.len() >= 2 {
            for i in 0..mentioned.len() {
                for j in (i + 1)..mentioned.len() {
                    let lhs = mentioned[i].clone();
                    let rhs = mentioned[j].clone();
                    let exists =
                        self.relations.lock().unwrap().iter().any(|r| {
                            (r.lhs == lhs && r.rhs == rhs) || (r.lhs == rhs && r.rhs == lhs)
                        });
                    if !exists {
                        let relation = Relation {
                            uuid: uuid("rel"),
                            name: "co_occurs".to_string(),
                            lhs: lhs.clone(),
                            rhs: rhs.clone(),
                            content: "mentioned together".to_string(),
                        };
                        added_relations.push(format!("{}<->{}", lhs, rhs));
                        self.relations.lock().unwrap().push(relation);
                    }
                }
            }
            // 持久化关系。
            let relations = self.relations.lock().unwrap();
            let lines: Vec<String> = relations
                .iter()
                .map(|r| serde_json::to_string(r).expect("serialize relation"))
                .collect();
            std::fs::write(self.dir.join("relations.jsonl"), lines.join("\n"))
                .map_err(|e| GraphMemoryError(format!("write relations: {e}")))?;
        }

        // 3) episode。
        let episode = Episode {
            uuid: uuid("ep"),
            content: content.to_string(),
            entities: mentioned.clone(),
        };
        let episode_uuid = episode.uuid.clone();
        Self::append(
            &self.dir.join("episodes.jsonl"),
            &serde_json::to_string(&episode)
                .map_err(|e| GraphMemoryError(format!("serialize episode: {e}")))?,
        )?;
        self.episodes.lock().unwrap().push(episode);

        // 实体回链 episode。
        {
            let mut entities = self.entities.lock().unwrap();
            for uuid in &mentioned {
                if let Some(entity) = entities.get_mut(uuid) {
                    entity.episodes.push(episode_uuid.clone());
                }
            }
        }

        Ok(AddMemoryResult {
            entities: added_entities,
            relations: added_relations,
            episode: episode_uuid,
        })
    }

    fn search(&self, query: &str) -> Result<Vec<GraphHit>, GraphMemoryError> {
        let lower = query.to_lowercase();
        let mut hits: Vec<GraphHit> = Vec::new();
        let entities = self.entities.lock().unwrap();
        for entity in entities.values() {
            let mut score = 0;
            if entity.name.to_lowercase().contains(&lower) {
                score += 3;
            }
            if entity.content.to_lowercase().contains(&lower) {
                score += 1;
            }
            if score > 0 {
                hits.push(GraphHit {
                    kind: "entity".to_string(),
                    uuid: entity.uuid.clone(),
                    name: entity.name.clone(),
                    content: entity.content.clone(),
                    score,
                });
            }
        }
        drop(entities);
        for relation in self.relations.lock().unwrap().iter() {
            if relation.name.to_lowercase().contains(&lower)
                || relation.content.to_lowercase().contains(&lower)
            {
                hits.push(GraphHit {
                    kind: "relation".to_string(),
                    uuid: relation.uuid.clone(),
                    name: relation.name.clone(),
                    content: relation.content.clone(),
                    score: 2,
                });
            }
        }
        for episode in self.episodes.lock().unwrap().iter() {
            if episode.content.to_lowercase().contains(&lower) {
                hits.push(GraphHit {
                    kind: "episode".to_string(),
                    uuid: episode.uuid.clone(),
                    name: String::new(),
                    content: episode.content.clone(),
                    score: 1,
                });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
        Ok(hits)
    }

    fn neighbors(&self, entity_uuid: &str) -> Result<Vec<Relation>, GraphMemoryError> {
        Ok(self
            .relations
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.lhs == entity_uuid || r.rhs == entity_uuid)
            .cloned()
            .collect())
    }

    fn entity_by_name(&self, name: &str) -> Option<Entity> {
        self.entity_by_normalized(name)
    }

    fn entities(&self) -> Vec<Entity> {
        let mut entities: Vec<Entity> = self.entities.lock().unwrap().values().cloned().collect();
        entities.sort_by(|a, b| a.name.cmp(&b.name));
        entities
    }

    fn relations(&self) -> Vec<Relation> {
        self.relations.lock().unwrap().clone()
    }

    fn episodes(&self) -> Vec<Episode> {
        let mut episodes = self.episodes.lock().unwrap().clone();
        episodes.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        episodes
    }
}

// ------------------------------------------------------------------
// 真实工具:graph_add_memory / graph_search / graph_neighbors
// ------------------------------------------------------------------

struct AddMemoryTool {
    graph: std::sync::Arc<dyn GraphMemory>,
}

#[async_trait]
impl Tool for AddMemoryTool {
    fn name(&self) -> &'static str {
        "graph_add_memory"
    }

    fn description(&self) -> &'static str {
        "add content to knowledge graph memory; arguments: {content}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "content": { "type": "string" } },
            "required": ["content"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field content".to_string()))?;
        let result = self.graph.add_memory(content).map_err(|e| ToolError(e.0))?;
        Ok(json!({
            "entities": result.entities,
            "relations": result.relations,
            "episode": result.episode,
        }))
    }
}

struct SearchTool {
    graph: std::sync::Arc<dyn GraphMemory>,
}

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &'static str {
        "graph_search"
    }

    fn description(&self) -> &'static str {
        "search knowledge graph memory; arguments: {query}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field query".to_string()))?;
        let hits = self.graph.search(query).map_err(|e| ToolError(e.0))?;
        Ok(json!({ "count": hits.len(), "hits": hits }))
    }
}

struct NeighborsTool {
    graph: std::sync::Arc<dyn GraphMemory>,
}

#[async_trait]
impl Tool for NeighborsTool {
    fn name(&self) -> &'static str {
        "graph_neighbors"
    }

    fn description(&self) -> &'static str {
        "list relations connected to an entity; arguments: {entity_name}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "entity_name": { "type": "string" } },
            "required": ["entity_name"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let name = arguments
            .get("entity_name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field entity_name".to_string()))?;
        let entity = self
            .graph
            .entity_by_name(name)
            .ok_or_else(|| ToolError(format!("entity not found: {name}")))?;
        let relations = self
            .graph
            .neighbors(&entity.uuid)
            .map_err(|e| ToolError(e.0))?;
        Ok(json!({ "count": relations.len(), "relations": relations }))
    }
}

/// 图谱记忆插件:提供 graph-memory seam + 三个真实工具。
pub struct GraphMemoryPlugin {
    dir: PathBuf,
}

impl GraphMemoryPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for GraphMemoryPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-graph-memory"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![GRAPH_MEMORY]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        let graph: std::sync::Arc<dyn GraphMemory> =
            std::sync::Arc::new(JsonlGraphMemory::open(self.dir.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?);
        let mut effects = vec![ctx.register(GRAPH_MEMORY, graph.clone())];
        effects.push(registry.register(std::sync::Arc::new(AddMemoryTool {
            graph: graph.clone(),
        })));
        effects.push(registry.register(std::sync::Arc::new(SearchTool {
            graph: graph.clone(),
        })));
        effects.push(registry.register(std::sync::Arc::new(NeighborsTool { graph })));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::graph_memory::GraphMemory;
    use ah_contracts::keys::GRAPH_MEMORY;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(GraphMemoryPlugin::new(root.join("graph"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn add_memory_extracts_entities_and_persists() {
        let root = std::env::temp_dir().join(format!("ah-gm-add-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let graph = ctx
            .service::<dyn GraphMemory>(&GRAPH_MEMORY)
            .expect("graph");

        let result = graph
            .add_memory("Alice works at OpenAI and Alice likes Python")
            .expect("add");
        eprintln!("DEBUG entities={:?}", result.entities);
        eprintln!("DEBUG all={:?}", graph.entities());
        assert!(!result.entities.is_empty(), "entities extracted");
        assert!(
            result.entities.contains(&"alice".to_string()),
            "normalized name"
        );
        assert!(
            graph.entity_by_name("Alice").is_some(),
            "case-insensitive lookup"
        );
        assert!(!result.episode.is_empty());

        // 持久化跨重开。
        drop(effects);
        let reopened = build_ctx(&root);
        let graph2 = reopened
            .0
            .service::<dyn GraphMemory>(&GRAPH_MEMORY)
            .expect("graph");
        assert!(graph2.entity_by_name("alice").is_some(), "persisted entity");
        assert_eq!(graph2.episodes().len(), 1, "persisted episode");
        drop(reopened.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn co_occurence_relations_and_neighbors() {
        let root = std::env::temp_dir().join(format!("ah-gm-neigh-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let graph = ctx
            .service::<dyn GraphMemory>(&GRAPH_MEMORY)
            .expect("graph");

        graph
            .add_memory("alice and bob discuss rust")
            .expect("add1");
        graph
            .add_memory("bob and carol discuss rust")
            .expect("add2");

        let bob = graph.entity_by_name("bob").expect("bob");
        let neighbors = graph.neighbors(&bob.uuid).expect("neighbors");
        assert!(!neighbors.is_empty(), "bob has relations");
        // 共现实体对(含全部抽取词);bob 至少与 alice 和 carol 各有一条。
        let all: std::collections::HashMap<String, String> = graph
            .entities()
            .into_iter()
            .map(|e| (e.uuid.clone(), e.name))
            .collect();
        let names: Vec<String> = neighbors
            .iter()
            .map(|r| {
                if r.lhs == bob.uuid {
                    all.get(&r.rhs).cloned().unwrap_or_default()
                } else {
                    all.get(&r.lhs).cloned().unwrap_or_default()
                }
            })
            .collect();
        assert!(names.iter().any(|n| n == "alice"), "bob-alice relation");
        assert!(names.iter().any(|n| n == "carol"), "bob-carol relation");

        // 同名合并:再提 alice 不新增实体。
        let before = graph.entities().len();
        graph.add_memory("alice").expect("add3");
        assert_eq!(graph.entities().len(), before, "alice merged, no dup");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn search_returns_ranked_hits() {
        let root = std::env::temp_dir().join(format!("ah-gm-search-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let graph = ctx
            .service::<dyn GraphMemory>(&GRAPH_MEMORY)
            .expect("graph");

        graph
            .add_memory("rust is a systems programming language")
            .expect("add");
        let hits = graph.search("rust").expect("search");
        assert!(!hits.is_empty(), "keyword hits");
        assert!(hits.iter().any(|h| h.kind == "entity"));
        // 无命中为空。
        assert!(graph.search("zzz-nonexistent").expect("search").is_empty());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn tools_roundtrip_via_registry() {
        let root = std::env::temp_dir().join(format!("ah-gm-tools-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let mut names = registry.names();
        names.sort();
        assert!(names.contains(&"graph_add_memory".to_string()));
        assert!(names.contains(&"graph_search".to_string()));
        assert!(names.contains(&"graph_neighbors".to_string()));

        registry
            .invoke(
                "graph_add_memory",
                json!({ "content": "openai releases gpt5" }),
            )
            .await
            .expect("add via tool");
        let search = registry
            .invoke("graph_search", json!({ "query": "gpt5" }))
            .await
            .expect("search via tool");
        assert!(search["count"].as_u64().unwrap_or(0) >= 1);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
