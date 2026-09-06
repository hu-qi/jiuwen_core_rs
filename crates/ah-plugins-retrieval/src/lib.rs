//! # ah-plugins-retrieval
//!
//! 真实知识库检索:文档分块 + BM25 风格词法统计 + 余弦向量检索。
//! 文档正文仍以 JSON 持久化；当 context 注册 KV seam 时，向量记录同步写入
//! `retrieval-vector:*` 外部索引。embedding 默认使用确定性本地向量，也支持
//! 通过 `credentials` seam 的 `dashscope.*` 凭据、`DASHSCOPE_*` 环境变量，或
//! `EMBEDDING_BASE_URL`/`EMBEDDING_MODEL` 使用真实外部 embedding 服务。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use ah_contracts::keys::{KV_STORE, RETRIEVAL, TOOLS};
use ah_contracts::prelude::Effect;
use ah_contracts::retrieval::{RetrievalError, RetrievalHit, RetrievalProvider};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::store::BaseKVStore;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 一个分块:文本 + 词频 + 向量(摄入时计算;旧文档为空则检索时补算)。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chunk {
    text: String,
    /// term -> 出现次数(小写词)。
    tokens: HashMap<String, u32>,
    #[serde(default)]
    embedding: Vec<f64>,
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

/// 向量维度(哈希 n-gram TF 稠密向量;文档注明为固定启发式维度)。
const EMBED_DIM: usize = 256;

/// FNV-1a 稳定哈希(与平台无关,保证跨运行一致)。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 特征:小写词 + 字符二元组(支持 CJK 与部分匹配)。
fn features(text: &str) -> Vec<String> {
    let mut feats = Vec::new();
    feats.extend(tokenize(text));
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    for pair in chars.windows(2) {
        let gram: String = pair.iter().collect();
        if gram.chars().any(|c| c.is_alphanumeric()) {
            feats.push(format!("bi:{gram}"));
        }
    }
    feats
}

/// embedding backend:本地确定性或外部 HTTP 实现。
pub trait EmbeddingBackend: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f64>, RetrievalError>;
}

struct LocalEmbeddingBackend;

impl EmbeddingBackend for LocalEmbeddingBackend {
    fn embed(&self, text: &str) -> Result<Vec<f64>, RetrievalError> {
        Ok(embed(text))
    }
}

/// 确定性本地 embedding:哈希特征到 [0, DIM) 的 TF 权重稠密向量。
fn embed(text: &str) -> Vec<f64> {
    let mut vector = vec![0.0f64; EMBED_DIM];
    for feature in features(text) {
        let idx = (fnv1a(feature.as_bytes()) % EMBED_DIM as u64) as usize;
        vector[idx] += 1.0;
    }
    vector
}

/// 余弦相似度(零向量返回 0)。
fn cosine(a: &[f64], b: &[f64]) -> f64 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
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

fn encode_index_part(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn vector_key(doc_id: &str, chunk_index: usize) -> String {
    format!(
        "retrieval-vector:{}:{chunk_index}",
        encode_index_part(doc_id)
    )
}

/// 检索 provider:文档正文保存在本地,向量可选地保存在 KV-backed 外部索引。
pub struct LocalRetrievalProvider {
    dir: PathBuf,
    docs: Mutex<HashMap<String, DocFile>>,
    embedder: std::sync::Arc<dyn EmbeddingBackend>,
    vector_store: Option<std::sync::Arc<dyn BaseKVStore>>,
}

impl LocalRetrievalProvider {
    /// 打开(或创建)知识库目录;默认使用本地确定性 embedding。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, RetrievalError> {
        Self::open_with_embedder_and_store(dir, std::sync::Arc::new(LocalEmbeddingBackend), None)
    }

    /// 打开知识库并注入 embedding backend。
    pub fn open_with_embedder(
        dir: impl Into<PathBuf>,
        embedder: std::sync::Arc<dyn EmbeddingBackend>,
    ) -> Result<Self, RetrievalError> {
        Self::open_with_embedder_and_store(dir, embedder, None)
    }

    /// 打开知识库并启用可选的外部 KV 向量索引。
    pub fn open_with_embedder_and_store(
        dir: impl Into<PathBuf>,
        embedder: std::sync::Arc<dyn EmbeddingBackend>,
        vector_store: Option<std::sync::Arc<dyn BaseKVStore>>,
    ) -> Result<Self, RetrievalError> {
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
            embedder,
            vector_store,
        })
    }

    fn path_for(&self, doc_id: &str) -> PathBuf {
        self.dir.join(format!("{doc_id}.json"))
    }
    fn persist_external_vectors(
        &self,
        doc_id: &str,
        chunks: &[Chunk],
    ) -> Result<(), RetrievalError> {
        let Some(store) = &self.vector_store else {
            return Ok(());
        };
        let prefix = format!("retrieval-vector:{}:", encode_index_part(doc_id));
        let old = store
            .scan(&prefix)
            .map_err(|e| RetrievalError(format!("vector index scan failed: {e}")))?;
        for entry in old {
            store
                .delete(&entry.key)
                .map_err(|e| RetrievalError(format!("vector index delete failed: {e}")))?;
        }
        for (index, chunk) in chunks.iter().enumerate() {
            store
                .set(
                    &vector_key(doc_id, index),
                    json!({ "doc_id": doc_id, "chunk_index": index, "embedding": chunk.embedding }),
                )
                .map_err(|e| RetrievalError(format!("vector index write failed: {e}")))?;
        }
        Ok(())
    }

    fn external_vector(
        &self,
        doc_id: &str,
        chunk_index: usize,
        local_chunk: &Chunk,
    ) -> Result<Vec<f64>, RetrievalError> {
        let Some(store) = &self.vector_store else {
            return if local_chunk.embedding.is_empty() {
                self.embedder.embed(&local_chunk.text)
            } else {
                Ok(local_chunk.embedding.clone())
            };
        };
        let value = store
            .get(&vector_key(doc_id, chunk_index))
            .map_err(|e| RetrievalError(format!("vector index read failed: {e}")))?
            .ok_or_else(|| {
                RetrievalError(format!(
                    "vector index entry missing for {doc_id}:{chunk_index}"
                ))
            })?;
        value
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| RetrievalError("vector index entry has no embedding".to_string()))?
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| {
                        RetrievalError("vector index contains invalid number".to_string())
                    })
            })
            .collect()
    }
    fn remove_external_vectors(&self, doc_id: &str) -> Result<(), RetrievalError> {
        let Some(store) = &self.vector_store else {
            return Ok(());
        };
        let prefix = format!("retrieval-vector:{}:", encode_index_part(doc_id));
        let entries = store
            .scan(&prefix)
            .map_err(|e| RetrievalError(format!("vector index scan failed: {e}")))?;
        for entry in entries {
            store
                .delete(&entry.key)
                .map_err(|e| RetrievalError(format!("vector index delete failed: {e}")))?;
        }
        Ok(())
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
        let chunks: Vec<Chunk> = chunk_text(text)
            .into_iter()
            .map(|text| {
                let mut tokens: HashMap<String, u32> = HashMap::new();
                for term in tokenize(&text) {
                    *tokens.entry(term).or_insert(0) += 1;
                }
                let embedding = self.embedder.embed(&text)?;
                Ok(Chunk {
                    text,
                    tokens,
                    embedding,
                })
            })
            .collect::<Result<_, RetrievalError>>()?;
        let doc = DocFile {
            doc_id: doc_id.to_string(),
            chunks,
            metadata,
        };
        self.persist_external_vectors(doc_id, &doc.chunks)?;
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

    fn retrieve_vector(&self, query: &str, k: usize) -> Vec<RetrievalHit> {
        self.retrieve_vector_checked(query, k).unwrap_or_default()
    }

    fn retrieve_vector_checked(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<RetrievalHit>, RetrievalError> {
        let docs = self.docs.lock().unwrap().clone();
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let query_vec = self.embedder.embed(query)?;
        let mut scored: Vec<RetrievalHit> = Vec::new();
        for doc in docs.values() {
            for (chunk_index, chunk) in doc.chunks.iter().enumerate() {
                let chunk_vec = self.external_vector(&doc.doc_id, chunk_index, chunk)?;
                let score = cosine(&query_vec, &chunk_vec);
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
        Ok(scored)
    }

    fn embedding(&self, text: &str) -> Vec<f64> {
        self.embedding_checked(text).unwrap_or_default()
    }

    fn embedding_checked(&self, text: &str) -> Result<Vec<f64>, RetrievalError> {
        self.embedder.embed(text)
    }

    fn remove(&self, doc_id: &str) -> Result<(), RetrievalError> {
        self.remove_external_vectors(doc_id)?;
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

/// 阿里云 DashScope 文本 embedding 客户端。
///
/// 使用 DashScope 原生 `/api/v1/services/embeddings/text-embedding/text-embedding`
/// 协议；请求、HTTP 状态、JSON 结构和向量数值错误均显式返回，不回退本地向量。
pub struct DashScopeEmbeddingClient {
    endpoint: String,
    model: String,
    api_key: String,
    max_retries: usize,
    retry_base_delay: std::time::Duration,
    request_lock: std::sync::Mutex<()>,
}

impl DashScopeEmbeddingClient {
    const DEFAULT_ENDPOINT: &'static str =
        "https://dashscope.aliyuncs.com/api/v1/services/embeddings/text-embedding/text-embedding";
    const DEFAULT_MODEL: &'static str = "text-embedding-v3";

    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, RetrievalError> {
        let endpoint = endpoint.into();
        let model = model.into();
        let api_key = api_key.into();
        if endpoint.trim().is_empty() || model.trim().is_empty() || api_key.trim().is_empty() {
            return Err(RetrievalError(
                "dashscope endpoint, model and api key are required".to_string(),
            ));
        }
        Ok(Self {
            endpoint,
            model,
            api_key,
            max_retries: 2,
            retry_base_delay: std::time::Duration::from_millis(100),
            request_lock: std::sync::Mutex::new(()),
        })
    }

    /// 配置 429/5xx 的有界指数退避重试。
    pub fn with_retry_policy(
        mut self,
        max_retries: usize,
        retry_base_delay: std::time::Duration,
    ) -> Self {
        self.max_retries = max_retries;
        self.retry_base_delay = retry_base_delay;
        self
    }

    /// 从 `DASHSCOPE_API_KEY` 构建；endpoint/model 支持环境变量覆盖。
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("DASHSCOPE_API_KEY").ok()?;
        let client = Self::new(
            std::env::var("DASHSCOPE_EMBEDDING_ENDPOINT")
                .unwrap_or_else(|_| Self::DEFAULT_ENDPOINT.to_string()),
            std::env::var("DASHSCOPE_EMBEDDING_MODEL")
                .unwrap_or_else(|_| Self::DEFAULT_MODEL.to_string()),
            api_key,
        )
        .ok()?;
        let max_retries = std::env::var("DASHSCOPE_MAX_RETRIES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(client.max_retries);
        let retry_base_delay = std::env::var("DASHSCOPE_RETRY_BASE_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(std::time::Duration::from_millis)
            .unwrap_or(client.retry_base_delay);
        Some(client.with_retry_policy(max_retries, retry_base_delay))
    }

    pub fn embed(&self, input: &str) -> Result<Vec<f64>, RetrievalError> {
        let body = serde_json::to_string(&json!({
            "model": self.model,
            "input": { "texts": [input] },
        }))
        .map_err(|e| RetrievalError(format!("DashScope request serialization failed: {e}")))?;
        let _request_guard = self.request_lock.lock().unwrap();
        let mut attempt = 0usize;
        let response = loop {
            let result = ureq::post(&self.endpoint)
                .set("Authorization", &format!("Bearer {}", self.api_key))
                .set("Content-Type", "application/json")
                .set("X-DashScope-Async", "disable")
                .send_string(&body);
            match result {
                Ok(response) => break response,
                Err(ureq::Error::Status(status, _))
                    if attempt < self.max_retries && (status == 429 || status >= 500) =>
                {
                    let delay = self
                        .retry_base_delay
                        .saturating_mul(1_u32 << attempt.min(10));
                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    attempt += 1;
                }
                Err(error) => {
                    return Err(RetrievalError(format!(
                        "DashScope embedding request failed after {} retries: {error}",
                        attempt
                    )));
                }
            }
        };
        let response_body = response
            .into_string()
            .map_err(|e| RetrievalError(format!("DashScope response read failed: {e}")))?;
        let value: Value = serde_json::from_str(&response_body)
            .map_err(|e| RetrievalError(format!("DashScope response JSON failed: {e}")))?;
        let values = value
            .get("output")
            .and_then(|output| output.get("embeddings"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("embedding"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                RetrievalError("DashScope response has no embedding vector".to_string())
            })?;
        if values.is_empty() {
            return Err(RetrievalError(
                "DashScope response vector is empty".to_string(),
            ));
        }
        values
            .iter()
            .map(|value| {
                let number = value.as_f64().ok_or_else(|| {
                    RetrievalError("DashScope vector contains non-number".to_string())
                })?;
                if number.is_finite() {
                    Ok(number)
                } else {
                    Err(RetrievalError(
                        "DashScope vector contains non-finite number".to_string(),
                    ))
                }
            })
            .collect()
    }
}

impl EmbeddingBackend for DashScopeEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f64>, RetrievalError> {
        DashScopeEmbeddingClient::embed(self, text)
    }
}

/// OpenAI-compatible 外部 embedding 客户端。
///
/// 该客户端只负责真实 HTTP 请求与响应校验；网络、HTTP、JSON 和维度错误均
/// 显式返回，不会退回本地哈希向量。
pub struct HttpEmbeddingClient {
    endpoint: String,
    model: String,
    api_key: Option<String>,
}

impl HttpEmbeddingClient {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: model.into(),
            api_key,
        }
    }

    pub fn embed(&self, input: &str) -> Result<Vec<f64>, RetrievalError> {
        if self.endpoint.trim().is_empty() || self.model.trim().is_empty() {
            return Err(RetrievalError(
                "embedding endpoint and model are required".to_string(),
            ));
        }
        let body = serde_json::to_string(&json!({ "input": input, "model": self.model }))
            .map_err(|e| RetrievalError(format!("embedding request serialization failed: {e}")))?;
        let mut request = ureq::post(&self.endpoint).set("Content-Type", "application/json");
        if let Some(api_key) = &self.api_key {
            request = request.set("Authorization", &format!("Bearer {api_key}"));
        }
        let response = request
            .send_string(&body)
            .map_err(|e| RetrievalError(format!("embedding request failed: {e}")))?;
        let response_body = response
            .into_string()
            .map_err(|e| RetrievalError(format!("embedding response read failed: {e}")))?;
        let value: Value = serde_json::from_str(&response_body)
            .map_err(|e| RetrievalError(format!("embedding response JSON failed: {e}")))?;
        let values = value
            .get("data")
            .and_then(Value::as_array)
            .and_then(|data| data.first())
            .and_then(|item| item.get("embedding"))
            .or_else(|| value.get("embedding"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                RetrievalError("embedding response has no embedding vector".to_string())
            })?;
        if values.is_empty() {
            return Err(RetrievalError(
                "embedding response vector is empty".to_string(),
            ));
        }
        values
            .iter()
            .map(|value| {
                let number = value.as_f64().ok_or_else(|| {
                    RetrievalError("embedding vector contains non-number".to_string())
                })?;
                if number.is_finite() {
                    Ok(number)
                } else {
                    Err(RetrievalError(
                        "embedding vector contains non-finite number".to_string(),
                    ))
                }
            })
            .collect()
    }
}

impl EmbeddingBackend for HttpEmbeddingClient {
    fn embed(&self, text: &str) -> Result<Vec<f64>, RetrievalError> {
        HttpEmbeddingClient::embed(self, text)
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
        "search the knowledge base; arguments: {query, k?, mode?} with mode bm25|vector"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "k": { "type": "integer" },
                "mode": { "type": "string", "enum": ["bm25", "vector"] },
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
        let mode = arguments
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("bm25");
        let hits = match mode {
            "vector" => self
                .retrieval
                .retrieve_vector_checked(query, k)
                .map_err(|e| ToolError(format!("vector search failed: {e}")))?,
            _ => self.retrieval.retrieve(query, k),
        };
        Ok(json!({ "mode": mode, "count": hits.len(), "hits": hits }))
    }
}

struct DashScopeSettings {
    endpoint: String,
    model: String,
    api_key: String,
}

fn dashscope_credentials(
    provider: Option<&dyn ah_contracts::credentials::CredentialProvider>,
) -> Option<DashScopeSettings> {
    let api_key = provider?.get("dashscope.api_key")?.value;
    let endpoint = std::env::var("DASHSCOPE_EMBEDDING_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            provider
                .and_then(|provider| provider.get("dashscope.base_url"))
                .map(|credential| credential.value)
        })
        .unwrap_or_else(|| DashScopeEmbeddingClient::DEFAULT_ENDPOINT.to_string());
    let model = std::env::var("DASHSCOPE_EMBEDDING_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            provider
                .and_then(|provider| provider.get("dashscope.model"))
                .map(|credential| credential.value)
        })
        .unwrap_or_else(|| DashScopeEmbeddingClient::DEFAULT_MODEL.to_string());
    Some(DashScopeSettings {
        endpoint,
        model,
        api_key,
    })
}

fn configured_embedder(ctx: &Context) -> std::sync::Arc<dyn EmbeddingBackend> {
    if let Ok(endpoint) = std::env::var("EMBEDDING_BASE_URL")
        && !endpoint.trim().is_empty()
    {
        let model = std::env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| "text-embedding-3-small".to_string());
        let api_key = std::env::var("EMBEDDING_API_KEY")
            .ok()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok());
        return std::sync::Arc::new(HttpEmbeddingClient::new(endpoint, model, api_key));
    }
    let credentials = ctx.service::<dyn ah_contracts::credentials::CredentialProvider>(
        &ah_contracts::keys::CREDENTIALS,
    );
    if let Some(config) = dashscope_credentials(credentials.as_deref())
        && let Ok(client) =
            DashScopeEmbeddingClient::new(config.endpoint, config.model, config.api_key)
    {
        return std::sync::Arc::new(client);
    }
    if let Some(client) = DashScopeEmbeddingClient::from_env() {
        return std::sync::Arc::new(client);
    }
    std::sync::Arc::new(LocalEmbeddingBackend)
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
        let vector_store = ctx.service::<dyn BaseKVStore>(&KV_STORE);
        let provider: std::sync::Arc<dyn RetrievalProvider> = std::sync::Arc::new(
            LocalRetrievalProvider::open_with_embedder_and_store(
                &self.dir,
                configured_embedder(ctx),
                vector_store,
            )
            .map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?,
        );
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
    fn external_vector_index_survives_reopen_and_remove() {
        use ah_contracts::store::{BaseKVStore, KvEntry, StoreError};
        use std::collections::HashMap;
        use std::sync::Arc;

        struct FakeStore(Mutex<HashMap<String, Value>>);
        impl Seam for FakeStore {}
        impl BaseKVStore for FakeStore {
            fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
                Ok(self.0.lock().unwrap().get(key).cloned())
            }
            fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
                self.0.lock().unwrap().insert(key.to_string(), value);
                Ok(())
            }
            fn delete(&self, key: &str) -> Result<(), StoreError> {
                self.0.lock().unwrap().remove(key);
                Ok(())
            }
            fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError> {
                Ok(self
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(key, _)| key.starts_with(prefix))
                    .map(|(key, value)| KvEntry {
                        key: key.clone(),
                        value: value.clone(),
                        updated_ms: 0,
                    })
                    .collect())
            }
        }

        let store: Arc<dyn BaseKVStore> = Arc::new(FakeStore(Mutex::new(HashMap::new())));
        let dir =
            std::env::temp_dir().join(format!("ah-retrieval-external-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider = LocalRetrievalProvider::open_with_embedder_and_store(
            &dir,
            Arc::new(LocalEmbeddingBackend),
            Some(store.clone()),
        )
        .expect("open");
        provider
            .ingest("doc", "Rust vector storage", json!({}))
            .expect("ingest");
        assert_eq!(store.scan("retrieval-vector:").unwrap().len(), 1);
        drop(provider);
        let reopened = LocalRetrievalProvider::open_with_embedder_and_store(
            &dir,
            Arc::new(LocalEmbeddingBackend),
            Some(store.clone()),
        )
        .expect("reopen");
        assert_eq!(
            reopened.retrieve_vector_checked("vector", 1).unwrap().len(),
            1
        );
        reopened.remove("doc").expect("remove");
        assert!(store.scan("retrieval-vector:").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn embedding_is_deterministic_and_dense() {
        let (provider, dir) = provider("embed");
        let v1 = provider.embedding("machine learning");
        let v2 = provider.embedding("machine learning");
        assert_eq!(v1, v2, "same text -> identical vector");
        assert_eq!(v1.len(), EMBED_DIM, "dense fixed-dimension vector");
        assert!(v1.iter().any(|x| *x > 0.0), "non-zero features");
        let v3 = provider.embedding("quantum physics");
        assert_ne!(v1, v3, "different text -> different vector");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_vector_finds_shared_terms_by_cosine() {
        let (provider, dir) = provider("vec");
        provider
            .ingest(
                "dogs",
                "Dogs are loyal pets. Puppies need daily care and training.",
                json!({}),
            )
            .expect("ingest dogs");
        provider
            .ingest(
                "stocks",
                "Stock markets rise and fall with interest rates and earnings.",
                json!({}),
            )
            .expect("ingest stocks");

        let hits = provider.retrieve_vector("puppy dog care", 2);
        assert!(!hits.is_empty(), "vector path returns hits");
        assert_eq!(hits[0].doc_id, "dogs", "semantic/shared-term match first");
        assert!(
            hits.iter().all(|h| (0.0..=1.0).contains(&h.score)),
            "score in [0,1]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_vector_handles_cjk_bigrams() {
        let (provider, dir) = provider("cjk");
        provider
            .ingest(
                "ml",
                "机器学习与自然语言处理是人工智能的核心方向。",
                json!({}),
            )
            .expect("ingest");
        let hits = provider.retrieve_vector("机器学习", 1);
        assert!(!hits.is_empty(), "CJK query matches via char bigrams");
        assert_eq!(hits[0].doc_id, "ml");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_knowledge_tool_supports_vector_mode() {
        use ah_contracts::keys::TOOLS;
        use ah_hub::plugin::DynPlugin;
        use std::sync::Arc as StdArc;

        let dir = std::env::temp_dir().join(format!("ah-retrieval-vecmode-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(RetrievalPlugin::new(&dir)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let _ = registry
            .invoke(
                "ingest_knowledge",
                json!({ "doc_id": "d1", "text": "Rust is a systems programming language." }),
            )
            .await
            .expect("ingest");
        let result = registry
            .invoke(
                "search_knowledge",
                json!({ "query": "rust", "mode": "vector" }),
            )
            .await
            .expect("search vector");
        assert_eq!(result["mode"], "vector");
        assert_eq!(result["count"], 1);

        drop(effects);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn dashscope_credentials_are_resolved_from_seam() {
        use ah_contracts::credentials::{Credential, CredentialError, CredentialProvider};

        struct StaticCredentials;
        impl Seam for StaticCredentials {}
        impl CredentialProvider for StaticCredentials {
            fn get(&self, name: &str) -> Option<Credential> {
                let value = match name {
                    "dashscope.api_key" => "credential-key",
                    "dashscope.base_url" => "http://credential-endpoint",
                    "dashscope.model" => "credential-model",
                    _ => return None,
                };
                Some(Credential {
                    name: name.to_string(),
                    value: value.to_string(),
                    source: "test".to_string(),
                })
            }
            fn set(&self, _: &str, _: &str, _: &str) -> Result<Credential, CredentialError> {
                unreachable!()
            }
            fn list(&self) -> Vec<Credential> {
                vec![]
            }
            fn remove(&self, _: &str) -> Result<(), CredentialError> {
                unreachable!()
            }
        }

        let config = dashscope_credentials(Some(&StaticCredentials)).expect("config");
        assert_eq!(config.endpoint, "http://credential-endpoint");
        assert_eq!(config.model, "credential-model");
        assert_eq!(config.api_key, "credential-key");
    }
}
#[test]
fn http_embedding_client_parses_openai_compatible_response() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let endpoint = format!("http://{}/v1/embeddings", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let size = stream.read(&mut chunk).expect("request");
            assert!(size > 0, "request closed before headers");
            request.extend_from_slice(&chunk[..size]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let body = r#"{"data":[{"embedding":[0.25,-0.5,1.0]}]}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("response");
        stream.flush().expect("flush response");
    });
    let client = HttpEmbeddingClient::new(endpoint, "test-model", Some("secret".into()));
    assert_eq!(
        client.embed("hello").expect("embedding"),
        vec![0.25, -0.5, 1.0]
    );
    server.join().expect("server");
}

#[test]
fn dashscope_embedding_client_posts_native_request_and_parses_response() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    assert!(DashScopeEmbeddingClient::new("", "model", "key").is_err());
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let size = stream.read(&mut buffer).expect("request");
            assert!(size > 0, "request closed before body");
            request.extend_from_slice(&buffer[..size]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("content length");
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        sender
            .send(String::from_utf8_lossy(&request).into_owned())
            .expect("capture request");
        let body = r#"{"output":{"embeddings":[{"embedding":[0.125,-0.25,0.5],"text_index":0}]}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("response");
    });
    let client =
        DashScopeEmbeddingClient::new(endpoint, "text-embedding-v3", "secret").expect("client");
    assert_eq!(
        client.embed("hello").expect("embedding"),
        vec![0.125, -0.25, 0.5]
    );
    let request = receiver.recv().expect("request capture");
    assert!(request.contains("Authorization: Bearer secret"));

    assert!(request.contains(r#""model":"text-embedding-v3""#));
    assert!(request.contains(r#""texts":["hello"]"#));
    server.join().expect("server");
}
#[test]
fn dashscope_embedding_client_retries_transient_status() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let server_calls = calls.clone();
    let server = thread::spawn(move || {
        for call in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let size = stream.read(&mut buffer).expect("request");
                assert!(size > 0, "request closed before body");
                request.extend_from_slice(&buffer[..size]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .expect("content length");
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            server_calls.fetch_add(1, Ordering::SeqCst);
            let (status, body) = if call == 0 {
                ("429 Too Many Requests", r#"{"error":"busy"}"#)
            } else {
                (
                    "200 OK",
                    r#"{"output":{"embeddings":[{"embedding":[1.0]}]}}"#,
                )
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("response");
        }
    });
    let client = DashScopeEmbeddingClient::new(endpoint, "text-embedding-v3", "secret")
        .expect("client")
        .with_retry_policy(1, std::time::Duration::from_millis(1));
    assert_eq!(client.embed("hello").expect("retry succeeds"), vec![1.0]);
    server.join().expect("server");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[test]
fn external_http_embedding_and_redis_vector_index_roundtrip() {
    use ah_contracts::store::BaseKVStore;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;

    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into());
    let redis = match ah_plugins_store::RedisKVStore::open(&redis_url) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            println!("skipping: redis unavailable: {error}");
            return;
        }
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind embedding fixture");
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("embedding request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let size = stream.read(&mut buffer).expect("read embedding request");
                assert!(size > 0, "embedding request closed before body");
                request.extend_from_slice(&buffer[..size]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .expect("content length");
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let body = r#"{"data":[{"embedding":[1.0,0.0]}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("embedding response");
        }
    });
    let dir = std::env::temp_dir().join(format!("ah-retrieval-external-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let provider = LocalRetrievalProvider::open_with_embedder_and_store(
        &dir,
        Arc::new(HttpEmbeddingClient::new(endpoint, "fixture", None)),
        Some(redis.clone()),
    )
    .expect("provider");
    let doc_id = format!("external-{}", std::process::id());
    provider
        .ingest(&doc_id, "external vector document", serde_json::json!({}))
        .expect("ingest");
    let hits = provider
        .retrieve_vector_checked("external vector query", 1)
        .expect("vector search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, doc_id);
    assert_eq!(
        redis
            .scan(&format!("retrieval-vector:{}:", encode_index_part(&doc_id)))
            .unwrap()
            .len(),
        1
    );
    provider.remove(&doc_id).expect("remove");
    assert!(
        redis
            .scan(&format!("retrieval-vector:{}:", encode_index_part(&doc_id)))
            .unwrap()
            .is_empty()
    );
    server.join().expect("fixture");
    let _ = std::fs::remove_dir_all(dir);
}
