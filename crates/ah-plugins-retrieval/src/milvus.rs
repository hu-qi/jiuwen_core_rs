//! Milvus REST v2 retrieval backend.
//!
//! The client intentionally does not maintain a local vector fallback. A
//! failed remote insert/search/delete is returned through the checked seam.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ah_contracts::retrieval::{RetrievalError, RetrievalHit, RetrievalProvider};
use ah_contracts::seam::Seam;
use serde_json::{Value, json};

use crate::EmbeddingBackend;

#[derive(Clone)]
struct Document {
    text: String,
}

/// Milvus HTTP JSON provider using the v2 vector database API.
pub struct MilvusRetrievalProvider {
    base_url: String,
    collection: String,
    token: Option<String>,
    embedder: std::sync::Arc<dyn EmbeddingBackend>,
    docs: Mutex<BTreeMap<String, Document>>,
    agent: ureq::Agent,
}

impl MilvusRetrievalProvider {
    pub fn new(
        base_url: impl Into<String>,
        collection: impl Into<String>,
        token: Option<String>,
        embedder: std::sync::Arc<dyn EmbeddingBackend>,
    ) -> Result<Self, RetrievalError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(RetrievalError("milvus URL must use http or https".into()));
        }
        let collection = collection.into();
        if collection.is_empty() {
            return Err(RetrievalError("milvus collection must not be empty".into()));
        }
        Ok(Self {
            base_url,
            collection,
            token,
            embedder,
            docs: Mutex::new(BTreeMap::new()),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(15))
                .build(),
        })
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let request = self
            .agent
            .request(method, &format!("{}{}", self.base_url, path));
        if let Some(token) = &self.token {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        }
    }

    fn post(&self, path: &str, payload: Value) -> Result<Value, RetrievalError> {
        self.request("POST", path)
            .set("Content-Type", "application/json")
            .send_json(payload)
            .map_err(|error| RetrievalError(format!("milvus POST {path}: {error}")))?
            .into_json()
            .map_err(|error| RetrievalError(format!("milvus POST {path} JSON: {error}")))
    }

    fn ensure_success(body: &Value, operation: &str) -> Result<(), RetrievalError> {
        if body.get("code").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Err(RetrievalError(format!(
                "milvus {operation} failed: {}",
                body.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            )));
        }
        Ok(())
    }
}

impl Seam for MilvusRetrievalProvider {}

impl RetrievalProvider for MilvusRetrievalProvider {
    fn ingest(&self, doc_id: &str, text: &str, metadata: Value) -> Result<(), RetrievalError> {
        if doc_id.is_empty() || text.trim().is_empty() {
            return Err(RetrievalError("doc_id and text must not be empty".into()));
        }
        let vector = self.embedder.embed(text)?;
        let body = self.post(
            "/v2/vectordb/entities/insert",
            json!({
                "collectionName": self.collection,
                "data": [{"doc_id": doc_id, "text": text, "vector": vector, "metadata": metadata}],
            }),
        )?;
        Self::ensure_success(&body, "insert")?;
        self.docs.lock().unwrap().insert(
            doc_id.to_string(),
            Document {
                text: text.to_string(),
            },
        );
        Ok(())
    }

    fn retrieve(&self, query: &str, k: usize) -> Vec<RetrievalHit> {
        let query_terms: Vec<String> = query
            .split_whitespace()
            .map(|word| word.to_lowercase())
            .collect();
        let mut hits: Vec<RetrievalHit> = self
            .docs
            .lock()
            .unwrap()
            .iter()
            .map(|(doc_id, document)| {
                let score = query_terms
                    .iter()
                    .filter(|term| document.text.to_lowercase().contains(term.as_str()))
                    .count() as f64;
                RetrievalHit {
                    doc_id: doc_id.clone(),
                    chunk: document.text.clone(),
                    score,
                }
            })
            .filter(|hit| hit.score > 0.0)
            .collect();
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        hits.truncate(k);
        hits
    }

    fn retrieve_vector(&self, query: &str, k: usize) -> Vec<RetrievalHit> {
        self.retrieve_vector_checked(query, k).unwrap_or_default()
    }

    fn retrieve_vector_checked(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<RetrievalHit>, RetrievalError> {
        let vector = self.embedder.embed(query)?;
        let body = self.post(
            "/v2/vectordb/entities/search",
            json!({
                "collectionName": self.collection,
                "data": [vector],
                "annsField": "vector",
                "limit": k,
                "outputFields": ["doc_id", "text"],
            }),
        )?;
        Self::ensure_success(&body, "search")?;
        let rows = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| RetrievalError("milvus search response missing data".into()))?;
        rows.iter()
            .map(|row| {
                Ok(RetrievalHit {
                    doc_id: row
                        .get("doc_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| RetrievalError("milvus hit missing doc_id".into()))?
                        .to_string(),
                    chunk: row
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| RetrievalError("milvus hit missing text".into()))?
                        .to_string(),
                    score: row
                        .get("distance")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| RetrievalError("milvus hit missing distance".into()))?,
                })
            })
            .collect()
    }

    fn embedding(&self, text: &str) -> Vec<f64> {
        self.embedder.embed(text).unwrap_or_default()
    }

    fn remove(&self, doc_id: &str) -> Result<(), RetrievalError> {
        let escaped_id = doc_id.replace('\'', "\\'");
        let body = self.post(
            "/v2/vectordb/entities/delete",
            json!({"collectionName": self.collection, "filter": format!("doc_id == '{escaped_id}'")}),
        )?;
        Self::ensure_success(&body, "delete")?;
        self.docs.lock().unwrap().remove(doc_id);
        Ok(())
    }

    fn doc_ids(&self) -> Vec<String> {
        self.docs.lock().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use std::sync::Arc;
    struct FixedEmbedder;
    impl EmbeddingBackend for FixedEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f64>, RetrievalError> {
            Ok(vec![1.0, 0.0])
        }
    }

    #[test]
    fn rest_roundtrip_uses_milvus_v2_entity_apis() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let size = stream.read(&mut chunk).expect("request");
                    request.extend_from_slice(&chunk[..size]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while request.len() < header_end + length {
                    let size = stream.read(&mut chunk).expect("request body");
                    request.extend_from_slice(&chunk[..size]);
                }
                let request = String::from_utf8_lossy(&request);
                let body = if request.starts_with("POST /v2/vectordb/entities/search") {
                    r#"{"code":0,"data":[{"doc_id":"doc-1","text":"milvus document","distance":0.95}]}"#
                } else {
                    r#"{"code":0,"data":{}}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let provider = MilvusRetrievalProvider::new(
            format!("http://{address}"),
            "agent_harness",
            Some("test-token".into()),
            Arc::new(FixedEmbedder),
        )
        .expect("provider");
        provider
            .ingest("doc-1", "milvus document", json!({"source":"test"}))
            .expect("insert");
        let hits = provider
            .retrieve_vector_checked("query", 1)
            .expect("search");
        assert_eq!(hits[0].doc_id, "doc-1");
        provider.remove("doc-1").expect("delete");
        assert!(provider.doc_ids().is_empty());
        handle.join().expect("server");
    }
}
