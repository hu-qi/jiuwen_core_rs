//! # ah-plugins-retrieval
//!
//! 真实本地知识库检索:文档分块 + 词法统计(BM25 风格),JSON 持久化。
//! 提供 retrieval seam + ingest_knowledge/search_knowledge 两个真实工具,
//! 让 agent 能通过工具调用检索(过工具执行管线,rails 同样生效)。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use ah_contracts::keys::{RETRIEVAL, TOOLS};
use ah_contracts::prelude::Effect;
use ah_contracts::retrieval::{RetrievalError, RetrievalHit, RetrievalProvider};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 一个分块:文本 + 词频。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chunk {
    text: String,
    /// term -> 出现次数(小写词)。
    tokens: HashMap<String, u32>,
}

/// 一篇文档:分块 + 元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocFile {
    doc_id: String,
    chunks: Vec<Chunk>,
    metadata: Value,
}

/// 分词:小写字母数字序列。
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && t.len() >= 2)
        .map(str::to_string)
        .collect()
}

/// 切块:按空行分段;段过大(>600 字符)再按固定窗口切。
fn chunk_text(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    for paragraph in text.split(
        "

",
    ) {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        if paragraph.chars().count() <= 600 {
            chunks.push(paragraph.to_string());
        } else {
            // 固定窗口切分(600 字符,无重叠)。
            let chars: Vec<char> = paragraph.chars().collect();
            for window in chars.chunks(600) {
                let piece: String = window.iter().collect();
                chunks.push(piece);
            }
        }
    }
    chunks
}

/// 真实本地检索 provider:dir/{doc_id}.json 持久化。
pub struct LocalRetrievalProvider {
    dir: PathBuf,
    docs: Mutex<HashMap<String, DocFile>>,
}

impl LocalRetrievalProvider {
    /// 打开(或创建)知识库目录;恢复已有文档。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, RetrievalError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| RetrievalError(format!("create retrieval dir failed: {e}")))?;
        let mut docs = HashMap::new();
        for entry in
            std::fs::read_dir(&dir).map_err(|e| RetrievalError(format!("read dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| RetrievalError(format!("entry failed: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(doc_id) = name.strip_suffix(".json")
                && let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(doc) = serde_json::from_str::<DocFile>(&text)
            {
                docs.insert(doc_id.to_string(), doc);
            }
        }
        Ok(Self {
            dir,
            docs: Mutex::new(docs),
        })
    }

    fn path_for(&self, doc_id: &str) -> PathBuf {
        self.dir.join(format!("{doc_id}.json"))
    }

    /// IDF:log(1 + N/df)。
    fn idf(&self, docs: &HashMap<String, DocFile>) -> HashMap<String, f64> {
        let n = docs.values().map(|d| d.chunks.len()).sum::<usize>().max(1);
        let mut df: HashMap<String, usize> = HashMap::new();
        for doc in docs.values() {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for chunk in &doc.chunks {
                for term in chunk.tokens.keys() {
                    if seen.insert(term.clone()) {
                        *df.entry(term.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        df.into_iter()
            .map(|(term, count)| (term, (1.0 + (n as f64) / (count as f64)).ln()))
            .collect()
    }
}

impl Seam for LocalRetrievalProvider {}

impl RetrievalProvider for LocalRetrievalProvider {
    fn ingest(&self, doc_id: &str, text: &str, metadata: Value) -> Result<(), RetrievalError> {
        let chunks = chunk_text(text)
            .into_iter()
            .map(|text| Chunk {
                tokens: {
                    let mut map: HashMap<String, u32> = HashMap::new();
                    for term in tokenize(&text) {
                        *map.entry(term).or_insert(0) += 1;
                    }
                    map
                },
                text,
            })
            .collect();
        let doc = DocFile {
            doc_id: doc_id.to_string(),
            chunks,
            metadata,
        };
        let text = serde_json::to_string(&doc)
            .map_err(|e| RetrievalError(format!("serialize failed: {e}")))?;
        std::fs::write(self.path_for(doc_id), text)
            .map_err(|e| RetrievalError(format!("write failed: {e}")))?;
        self.docs.lock().unwrap().insert(doc_id.to_string(), doc);
        Ok(())
    }

    fn retrieve(&self, query: &str, k: usize) -> Vec<RetrievalHit> {
        let docs = self.docs.lock().unwrap().clone();
        if docs.is_empty() {
            return Vec::new();
        }
        let idf = self.idf(&docs);
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<RetrievalHit> = Vec::new();
        for doc in docs.values() {
            for chunk in &doc.chunks {
                let mut score = 0.0f64;
                for term in &query_terms {
                    if let Some(tf) = chunk.tokens.get(term) {
                        let idf_val = idf.get(term).copied().unwrap_or(0.0);
                        score += (*tf as f64) * idf_val;
                    }
                }
                if score > 0.0 {
                    scored.push(RetrievalHit {
                        doc_id: doc.doc_id.clone(),
                        chunk: chunk.text.clone(),
                        score,
                    });
                }
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        scored
    }

    fn remove(&self, doc_id: &str) -> Result<(), RetrievalError> {
        let path = self.path_for(doc_id);
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| RetrievalError(format!("remove failed: {e}")))?;
        }
        self.docs.lock().unwrap().remove(doc_id);
        Ok(())
    }

    fn doc_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.docs.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// ingest_knowledge 工具:摄入一篇文档。
pub struct IngestKnowledgeTool {
    retrieval: std::sync::Arc<dyn RetrievalProvider>,
}

impl IngestKnowledgeTool {
    pub fn new(retrieval: std::sync::Arc<dyn RetrievalProvider>) -> Self {
        Self { retrieval }
    }
}

#[async_trait]
impl Tool for IngestKnowledgeTool {
    fn name(&self) -> &'static str {
        "ingest_knowledge"
    }

    fn description(&self) -> &'static str {
        "ingest a document into the knowledge base; arguments: {doc_id, text}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "doc_id": { "type": "string" },
                "text": { "type": "string" },
            },
            "required": ["doc_id", "text"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let doc_id = arguments
            .get("doc_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field doc_id".to_string()))?;
        let text = arguments
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field text".to_string()))?;
        self.retrieval
            .ingest(doc_id, text, json!({}))
            .map_err(|e| ToolError(format!("ingest failed: {e}")))?;
        Ok(json!({ "doc_id": doc_id, "ingested": true }))
    }
}

/// search_knowledge 工具:检索知识库。
pub struct SearchKnowledgeTool {
    retrieval: std::sync::Arc<dyn RetrievalProvider>,
}

impl SearchKnowledgeTool {
    pub fn new(retrieval: std::sync::Arc<dyn RetrievalProvider>) -> Self {
        Self { retrieval }
    }
}

#[async_trait]
impl Tool for SearchKnowledgeTool {
    fn name(&self) -> &'static str {
        "search_knowledge"
    }

    fn description(&self) -> &'static str {
        "search the knowledge base; arguments: {query, k?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "k": { "type": "integer" },
            },
            "required": ["query"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field query".to_string()))?;
        let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(3) as usize;
        let hits = self.retrieval.retrieve(query, k);
        Ok(json!({ "count": hits.len(), "hits": hits }))
    }
}

/// 检索插件:提供 retrieval seam,并注册 ingest_knowledge/search_knowledge 工具。
pub struct RetrievalPlugin {
    dir: PathBuf,
}

impl RetrievalPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for RetrievalPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-retrieval"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RETRIEVAL]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: std::sync::Arc<dyn RetrievalProvider> =
            std::sync::Arc::new(LocalRetrievalProvider::open(&self.dir).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?);
        let mut effects = vec![ctx.register(RETRIEVAL, provider.clone())];

        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(
            registry.register(std::sync::Arc::new(IngestKnowledgeTool::new(
                provider.clone(),
            ))),
        );
        effects.push(registry.register(std::sync::Arc::new(SearchKnowledgeTool::new(provider))));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(tag: &str) -> (LocalRetrievalProvider, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ah-retrieval-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider = LocalRetrievalProvider::open(&dir).expect("open");
        (provider, dir)
    }

    #[test]
    fn ingest_and_retrieve_hits_relevant_doc() {
        let (provider, dir) = provider("hit");
        provider
            .ingest(
                "rust",
                "Rust is a systems programming language with memory safety.",
                json!({}),
            )
            .expect("ingest rust");
        provider
            .ingest(
                "cooking",
                "Cooking pasta requires boiling water and salt.",
                json!({}),
            )
            .expect("ingest cooking");

        let hits = provider.retrieve("rust memory safety", 3);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].doc_id, "rust", "最相关文档应排第一");
        assert!(hits[0].chunk.contains("Rust"));

        // 无关查询无命中。
        assert!(provider.retrieve("quantum physics", 3).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_across_reopen() {
        let (provider, dir) = provider("persist");
        provider
            .ingest("kb", "The capital of France is Paris.", json!({}))
            .expect("ingest");
        drop(provider);

        let reopened = LocalRetrievalProvider::open(&dir).expect("reopen");
        assert_eq!(reopened.doc_ids(), vec!["kb".to_string()]);
        let hits = reopened.retrieve("paris", 1);
        assert_eq!(hits.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retrieval_plugin_registers_tools() {
        use ah_contracts::keys::TOOLS;
        use ah_hub::plugin::DynPlugin;
        use std::sync::Arc as StdArc;

        let dir = std::env::temp_dir().join(format!("ah-retrieval-plugin-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(RetrievalPlugin::new(&dir)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let mut names = registry.names();
        names.sort();
        assert_eq!(names, vec!["ingest_knowledge", "search_knowledge"]);

        // 工具真实调用:ingest -> search 往返(真实文件落盘)。
        let _ = registry
            .invoke(
                "ingest_knowledge",
                json!({ "doc_id": "d1", "text": "OpenJiuwen is an agent framework." }),
            )
            .await
            .expect("ingest");
        let result = registry
            .invoke("search_knowledge", json!({ "query": "agent framework" }))
            .await
            .expect("search");
        assert_eq!(result["count"], 1);
        assert!(dir.join("d1.json").exists(), "知识库真实落盘");

        drop(effects);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
