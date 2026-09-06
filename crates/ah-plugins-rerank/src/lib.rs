//! # ah-plugins-rerank
//!
//! 真实检索重排器(对齐 retrieval reranker):
//! - 把词法候选与向量候选按 doc_id+chunk 合并;
//! - 每条候选分数 = lexical_weight × bm25_norm + vector_weight × cosine_norm
//!   (两条路径各自按原始 score 在候选内归一化到 [0,1]);
//! - 多样性惩罚:已选结果与候选共享词(按空白分词)时按系数扣分;
//! - 按融合分数降序输出 top-k。确定性可测。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ah_contracts::credentials::CredentialProvider;
use ah_contracts::keys::{QUERY_RERANK, RERANK};
use ah_contracts::prelude::Effect;
use ah_contracts::rerank::{QueryReranker, RerankConfig, RerankError, RerankedHit, Reranker};
use ah_contracts::retrieval::RetrievalHit;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 合并候选:key = doc_id + "\x00" + chunk。
fn merge_candidates(
    lexical: &[RetrievalHit],
    vector: &[RetrievalHit],
) -> Vec<(String, String, f64, f64)> {
    let mut map: BTreeMap<String, (String, String, f64, f64)> = BTreeMap::new();
    // 词法路径:记录原始分数(用于归一化)。
    let lex_max = lexical
        .iter()
        .map(|h| h.score)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    for hit in lexical {
        let key = format!("{}\x00{}", hit.doc_id, hit.chunk);
        let norm = hit.score / lex_max;
        match map.get_mut(&key) {
            Some(entry) => entry.2 = norm,
            None => {
                map.insert(key, (hit.doc_id.clone(), hit.chunk.clone(), norm, 0.0));
            }
        }
    }
    let vec_max = vector
        .iter()
        .map(|h| h.score)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    for hit in vector {
        let key = format!("{}\x00{}", hit.doc_id, hit.chunk);
        let norm = hit.score / vec_max;
        match map.get_mut(&key) {
            Some(entry) => entry.3 = norm,
            None => {
                map.insert(key, (hit.doc_id.clone(), hit.chunk.clone(), 0.0, norm));
            }
        }
    }
    map.into_values().collect()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn dashscope_rerank_key(provider: Option<&dyn CredentialProvider>) -> Option<String> {
    provider?
        .get("dashscope.api_key")
        .map(|credential| credential.value)
        .filter(|value| !value.trim().is_empty())
}

/// DashScope 原生文本重排客户端。
///
/// 外部 rerank 必须保留 query；因此通过 `QueryReranker` 暴露，不错误复用
/// 不带 query 的历史融合接口。
pub struct DashScopeReranker {
    endpoint: String,
    model: String,
    api_key: String,
    request_lock: Mutex<()>,
}

impl DashScopeReranker {
    const DEFAULT_ENDPOINT: &'static str =
        "https://dashscope.aliyuncs.com/api/v1/services/rerank/text-rerank";
    const DEFAULT_MODEL: &'static str = "gte-rerank-v2";

    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Result<Self, RerankError> {
        let endpoint = endpoint.into();
        let model = model.into();
        let api_key = api_key.into();
        if endpoint.trim().is_empty() || model.trim().is_empty() || api_key.trim().is_empty() {
            return Err(RerankError(
                "dashscope rerank endpoint, model and api key are required".to_string(),
            ));
        }
        Ok(Self {
            endpoint,
            model,
            api_key,
            request_lock: Mutex::new(()),
        })
    }

    pub fn from_env() -> Result<Option<Self>, RerankError> {
        let Some(api_key) = std::env::var("DASHSCOPE_API_KEY").ok() else {
            return Ok(None);
        };
        Self::new(
            std::env::var("DASHSCOPE_RERANK_ENDPOINT")
                .unwrap_or_else(|_| Self::DEFAULT_ENDPOINT.to_string()),
            std::env::var("DASHSCOPE_RERANK_MODEL")
                .unwrap_or_else(|_| Self::DEFAULT_MODEL.to_string()),
            api_key,
        )
        .map(Some)
    }
}

impl Seam for DashScopeReranker {}

impl QueryReranker for DashScopeReranker {
    fn rerank_query(
        &self,
        query: &str,
        candidates: &[RetrievalHit],
        k: usize,
    ) -> Result<Vec<RerankedHit>, RerankError> {
        if query.trim().is_empty() {
            return Err(RerankError("rerank query must not be empty".to_string()));
        }
        if candidates.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let body = serde_json::to_string(&serde_json::json!({
            "model": self.model,
            "input": { "query": query, "documents": candidates.iter().map(|hit| &hit.chunk).collect::<Vec<_>>() },
        }))
        .map_err(|e| RerankError(format!("DashScope rerank request serialization failed: {e}")))?;
        let _request_guard = self.request_lock.lock().unwrap();
        let response = ureq::post(&self.endpoint)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set("X-DashScope-Async", "disable")
            .send_string(&body)
            .map_err(|e| RerankError(format!("DashScope rerank request failed: {e}")))?;
        let raw = response
            .into_string()
            .map_err(|e| RerankError(format!("DashScope rerank response read failed: {e}")))?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| RerankError(format!("DashScope rerank response JSON failed: {e}")))?;
        let results = value
            .get("output")
            .and_then(|output| output.get("results"))
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| RerankError("DashScope rerank response has no results".to_string()))?;
        let mut ranked = Vec::with_capacity(results.len());
        for result in results {
            let index = result
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| RerankError("DashScope rerank result has no index".to_string()))?
                as usize;
            let score = result
                .get("relevance_score")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| RerankError("DashScope rerank result has no score".to_string()))?;
            if !score.is_finite() {
                return Err(RerankError(
                    "DashScope rerank score is not finite".to_string(),
                ));
            }
            let candidate = candidates.get(index).ok_or_else(|| {
                RerankError(format!("DashScope rerank index out of range: {index}"))
            })?;
            ranked.push(RerankedHit {
                doc_id: candidate.doc_id.clone(),
                chunk: candidate.chunk.clone(),
                score: score.clamp(0.0, 1.0),
            });
        }
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(k);
        Ok(ranked)
    }
}

/// 真实重排器。
pub struct HybridReranker;

impl Seam for HybridReranker {}

impl Reranker for HybridReranker {
    fn rerank(
        &self,
        lexical: &[RetrievalHit],
        vector: &[RetrievalHit],
        k: usize,
        config: &RerankConfig,
    ) -> Result<Vec<RerankedHit>, RerankError> {
        if lexical.is_empty() && vector.is_empty() {
            return Err(RerankError("no candidates".to_string()));
        }
        if config.lexical_weight < 0.0 || config.vector_weight < 0.0 {
            return Err(RerankError("weights must be non-negative".to_string()));
        }
        let mut candidates = merge_candidates(lexical, vector);
        // 多样性惩罚:贪心选前 k,与已选共享词扣分。
        let mut selected: Vec<RerankedHit> = Vec::new();
        let mut selected_words: Vec<std::collections::HashSet<String>> = Vec::new();
        while !candidates.is_empty() && selected.len() < k {
            let mut best_idx = 0usize;
            let mut best_score = f64::NEG_INFINITY;
            for (idx, (_doc_id, chunk, lex_norm, vec_norm)) in candidates.iter().enumerate() {
                let fused = config.lexical_weight * lex_norm + config.vector_weight * vec_norm;
                // 多样性:与已选结果的共享词数惩罚。
                let chunk_words: std::collections::HashSet<String> =
                    words(chunk).into_iter().collect();
                let overlap: usize = selected_words
                    .iter()
                    .map(|sel| chunk_words.intersection(sel).count())
                    .sum();
                let score = fused - config.diversity_penalty * overlap as f64;
                if score > best_score {
                    best_score = score;
                    best_idx = idx;
                }
            }
            let (doc_id, chunk, lex_norm, vec_norm) = candidates.remove(best_idx);
            let fused = config.lexical_weight * lex_norm + config.vector_weight * vec_norm;
            selected.push(RerankedHit {
                doc_id: doc_id.clone(),
                chunk: chunk.clone(),
                score: fused.clamp(0.0, 1.0),
            });
            selected_words.push(words(&chunk).into_iter().collect());
        }
        Ok(selected)
    }
}

impl QueryReranker for HybridReranker {
    fn rerank_query(
        &self,
        _query: &str,
        candidates: &[RetrievalHit],
        k: usize,
    ) -> Result<Vec<RerankedHit>, RerankError> {
        self.rerank(
            candidates,
            &[],
            k,
            &RerankConfig {
                lexical_weight: 1.0,
                vector_weight: 0.0,
                ..RerankConfig::default()
            },
        )
    }
}

/// rerank 插件:提供 rerank seam。
pub struct RerankPlugin;

impl Plugin for RerankPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rerank"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RERANK, QUERY_RERANK]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let reranker: Arc<dyn Reranker> = Arc::new(HybridReranker);
        let credentials = ctx.service::<dyn CredentialProvider>(&ah_contracts::keys::CREDENTIALS);
        let query_reranker: Arc<dyn QueryReranker> =
            if let Some(api_key) = dashscope_rerank_key(credentials.as_deref()) {
                let endpoint = std::env::var("DASHSCOPE_RERANK_ENDPOINT")
                    .unwrap_or_else(|_| DashScopeReranker::DEFAULT_ENDPOINT.to_string());
                let model = std::env::var("DASHSCOPE_RERANK_MODEL")
                    .unwrap_or_else(|_| DashScopeReranker::DEFAULT_MODEL.to_string());
                Arc::new(
                    DashScopeReranker::new(endpoint, model, api_key).map_err(|error| {
                        PluginError::Apply {
                            plugin: self.name(),
                            message: error.0,
                        }
                    })?,
                )
            } else {
                match DashScopeReranker::from_env() {
                    Ok(Some(client)) => Arc::new(client),
                    Ok(None) => Arc::new(HybridReranker),
                    Err(error) => {
                        return Err(PluginError::Apply {
                            plugin: self.name(),
                            message: error.0,
                        });
                    }
                }
            };
        Ok(vec![
            ctx.register(RERANK, reranker),
            ctx.register(QUERY_RERANK, query_reranker),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{QUERY_RERANK, RERANK};
    use ah_contracts::rerank::{QueryReranker, Reranker};
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn hit(doc: &str, chunk: &str, score: f64) -> RetrievalHit {
        RetrievalHit {
            doc_id: doc.to_string(),
            chunk: chunk.to_string(),
            score,
        }
    }

    #[test]
    fn rerank_fuses_lexical_and_vector_scores() {
        let reranker = HybridReranker;
        // doc A:词法 1.0 / 向量 0.5;doc B:词法 0.5 / 向量 1.0 → 等权重融合同为 0.75。
        let lexical = vec![hit("a", "alpha beta", 1.0), hit("b", "beta gamma", 0.5)];
        let vector = vec![hit("a", "alpha beta", 0.5), hit("b", "beta gamma", 1.0)];
        let result = reranker
            .rerank(&lexical, &vector, 10, &RerankConfig::default())
            .expect("rerank");
        assert_eq!(result.len(), 2);
        let a = result.iter().find(|r| r.doc_id == "a").expect("a");
        let b = result.iter().find(|r| r.doc_id == "b").expect("b");
        assert!((a.score - 0.75).abs() < 1e-9, "fused 0.75: {}", a.score);
        assert!((b.score - 0.75).abs() < 1e-9, "fused 0.75: {}", b.score);
    }

    #[test]
    fn diversity_penalty_reorders_redundant_candidates() {
        let reranker = HybridReranker;
        // 三个候选都与 query 相关;前两个共享词多 → 第三个因多样性上升。
        let lexical = vec![
            hit("d1", "rust compiler borrow checker", 1.0),
            hit("d2", "rust compiler borrow rules", 0.9),
            hit("d3", "cargo build system", 0.6),
        ];
        let config = RerankConfig {
            lexical_weight: 1.0,
            vector_weight: 0.0,
            diversity_penalty: 0.3,
        };
        let result = reranker.rerank(&lexical, &[], 3, &config).expect("rerank");
        assert_eq!(result.len(), 3);
        // d1 最高分仍第一。
        assert_eq!(result[0].doc_id, "d1");
        // 多样性惩罚使 d3(与 d1 共享少)比 d2 更早(0.6 vs 0.9-0.3×overlap)。
        let d3_pos = result.iter().position(|r| r.doc_id == "d3").expect("d3");
        assert!(
            d3_pos <= 1,
            "diversity promotes distinct candidate: {result:?}"
        );
    }

    #[test]
    fn errors_on_empty_and_bad_weights() {
        let reranker = HybridReranker;
        assert!(
            reranker
                .rerank(&[], &[], 3, &RerankConfig::default())
                .is_err(),
            "no candidates"
        );
        let bad = RerankConfig {
            lexical_weight: -0.1,
            ..Default::default()
        };
        let lexical = vec![hit("a", "x", 1.0)];
        assert!(
            reranker.rerank(&lexical, &[], 3, &bad).is_err(),
            "negative weight"
        );
    }

    #[test]
    fn plugin_registers_reranker() {
        let ctx = Context::new();
        let plugin: DynPlugin = StdArc::new(RerankPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let reranker = ctx.service::<dyn Reranker>(&RERANK).expect("rerank");
        let query_reranker = ctx
            .service::<dyn QueryReranker>(&QUERY_RERANK)
            .expect("query rerank");
        let lexical = vec![hit("a", "hello world", 1.0)];
        let result = reranker
            .rerank(&lexical, &[], 1, &RerankConfig::default())
            .expect("rerank");
        assert_eq!(result.len(), 1);
        let query_result = query_reranker
            .rerank_query("hello", &lexical, 1)
            .expect("query rerank");
        assert_eq!(query_result.len(), 1);
        drop(effects);
        assert!(!ctx.has_service(&RERANK));
        assert!(!ctx.has_service(&QUERY_RERANK));
    }
    #[test]
    fn dashscope_reranker_preserves_query_and_maps_scores() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;
        use std::thread;

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
            let body = r#"{"output":{"results":[{"index":1,"relevance_score":0.95},{"index":0,"relevance_score":0.25}]}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("response");
        });
        let reranker = DashScopeReranker::new(endpoint, "gte-rerank-v2", "secret").expect("client");
        let candidates = vec![hit("a", "first", 0.0), hit("b", "second", 0.0)];
        let result = reranker
            .rerank_query("find second", &candidates, 2)
            .expect("rerank");
        assert_eq!(result[0].doc_id, "b");
        assert_eq!(result[0].score, 0.95);
        let request = receiver.recv().expect("request capture");
        assert!(request.contains("Authorization: Bearer secret"));
        assert!(request.contains(r#""query":"find second""#));
        assert!(request.contains(r#""documents":["first","second"]"#));
        server.join().expect("server");
    }
    #[test]
    fn dashscope_reranker_serializes_concurrent_requests() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::thread;
        use std::time::Duration;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let server_active = active.clone();
        let server_maximum = maximum.clone();
        let server = thread::spawn(move || {
            let mut workers = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let current_active = server_active.clone();
                let current_maximum = server_maximum.clone();
                workers.push(thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 2048];
                    loop {
                        let size = stream.read(&mut buffer).expect("request");
                        assert!(size > 0, "request closed before body");
                        request.extend_from_slice(&buffer[..size]);
                        let Some(header_end) =
                            request.windows(4).position(|window| window == b"\r\n\r\n")
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
                    let now = current_active.fetch_add(1, Ordering::SeqCst) + 1;
                    current_maximum.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    let body = r#"{"output":{"results":[{"index":0,"relevance_score":0.5}]}}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .expect("response");
                    current_active.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for worker in workers {
                worker.join().expect("worker");
            }
        });
        let reranker =
            Arc::new(DashScopeReranker::new(endpoint, "gte-rerank-v2", "secret").expect("client"));
        let candidates = Arc::new(vec![hit("a", "first", 0.0)]);
        let workers = (0..2)
            .map(|_| {
                let reranker = reranker.clone();
                let candidates = candidates.clone();
                thread::spawn(move || {
                    reranker
                        .rerank_query("find", &candidates, 1)
                        .expect("rerank");
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("client worker");
        }
        server.join().expect("server");
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn dashscope_rerank_key_uses_credentials_seam() {
        use ah_contracts::credentials::{Credential, CredentialError, CredentialProvider};

        struct StaticCredentials;
        impl Seam for StaticCredentials {}
        impl CredentialProvider for StaticCredentials {
            fn get(&self, name: &str) -> Option<Credential> {
                (name == "dashscope.api_key").then(|| Credential {
                    name: name.to_string(),
                    value: "credential-key".to_string(),
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

        assert_eq!(
            dashscope_rerank_key(Some(&StaticCredentials)),
            Some("credential-key".to_string())
        );
    }
}
