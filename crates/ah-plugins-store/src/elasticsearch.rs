//! Elasticsearch REST KV backend.
//!
//! Documents are stored in an Elasticsearch index using the `_doc` API. The
//! `key` field remains the source of truth; document ids are hex encoded so
//! arbitrary application keys cannot alter the request path.

use ah_contracts::keys::KV_STORE;
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::seam::Seam;
use ah_contracts::store::{BaseKVStore, KvEntry, StoreError};
use serde_json::{Value, json};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn encode_id(key: &str) -> String {
    key.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Elasticsearch JSON REST implementation of `BaseKVStore`.
pub struct ElasticsearchKVStore {
    base_url: String,
    index: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl ElasticsearchKVStore {
    /// Create a client for an Elasticsearch URL, for example
    /// `https://elastic.example:9200`.
    pub fn new(
        base_url: impl Into<String>,
        index: impl Into<String>,
        api_key: Option<String>,
    ) -> Result<Self, StoreError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(StoreError(
                "elasticsearch URL must use http or https".into(),
            ));
        }
        let index = index.into();
        if index.is_empty() || index.contains('/') {
            return Err(StoreError(
                "elasticsearch index must be a non-empty path segment".into(),
            ));
        }
        Ok(Self {
            base_url,
            index,
            api_key,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(10))
                .build(),
        })
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let request = self
            .agent
            .request(method, &format!("{}{path}", self.base_url));
        if let Some(api_key) = &self.api_key {
            request.set("Authorization", &format!("ApiKey {api_key}"))
        } else {
            request
        }
    }

    fn document_path(&self, key: &str) -> String {
        format!("/{}/_doc/{}", self.index, encode_id(key))
    }

    fn error(operation: &str, error: ureq::Error) -> StoreError {
        StoreError(format!("elasticsearch {operation}: {error}"))
    }

    fn parse_source(operation: &str, body: Value) -> Result<KvEntry, StoreError> {
        let source = body
            .get("_source")
            .cloned()
            .ok_or_else(|| StoreError(format!("elasticsearch {operation}: missing _source")))?;
        serde_json::from_value(source)
            .map_err(|error| StoreError(format!("elasticsearch {operation} source: {error}")))
    }
}

impl Seam for ElasticsearchKVStore {}

impl BaseKVStore for ElasticsearchKVStore {
    fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
        let response = match self.request("GET", &self.document_path(key)).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(error) => return Err(Self::error("GET", error)),
        };
        let body: Value = response
            .into_json()
            .map_err(|error| StoreError(format!("elasticsearch GET JSON: {error}")))?;
        Ok(Some(Self::parse_source("GET", body)?.value))
    }

    fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
        let entry = KvEntry {
            key: key.to_string(),
            value,
            updated_ms: now_ms(),
        };
        let response = self
            .request("PUT", &self.document_path(key))
            .set("Content-Type", "application/json")
            .send_json(json!({
                "key": entry.key,
                "value": entry.value,
                "updated_ms": entry.updated_ms,
            }))
            .map_err(|error| Self::error("PUT", error))?;
        if !(200..300).contains(&response.status()) {
            return Err(StoreError(format!(
                "elasticsearch PUT returned {}",
                response.status()
            )));
        }
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        match self.request("DELETE", &self.document_path(key)).call() {
            Ok(_) | Err(ureq::Error::Status(404, _)) => Ok(()),
            Err(error) => Err(Self::error("DELETE", error)),
        }
    }

    fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError> {
        let response = self
            .request("POST", &format!("/{}/_search", self.index))
            .set("Content-Type", "application/json")
            .send_json(json!({
                "query": {"prefix": {"key": prefix}},
                "sort": [{"key": "asc"}],
                "size": 10000,
            }))
            .map_err(|error| Self::error("SEARCH", error))?;
        let body: Value = response
            .into_json()
            .map_err(|error| StoreError(format!("elasticsearch SEARCH JSON: {error}")))?;
        let hits = body
            .pointer("/hits/hits")
            .and_then(Value::as_array)
            .ok_or_else(|| StoreError("elasticsearch SEARCH: missing hits.hits".into()))?;
        hits.iter()
            .cloned()
            .map(|hit| Self::parse_source("SEARCH", hit))
            .collect()
    }
}

/// Plugin wrapper for mounting an Elasticsearch KV service in a profile.
pub struct ElasticsearchStorePlugin {
    base_url: String,
    index: String,
    api_key: Option<String>,
}

impl ElasticsearchStorePlugin {
    pub fn new(
        base_url: impl Into<String>,
        index: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            index: index.into(),
            api_key,
        }
    }
}

impl Plugin for ElasticsearchStorePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-store-elasticsearch"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![KV_STORE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let store = ElasticsearchKVStore::new(
            self.base_url.clone(),
            self.index.clone(),
            self.api_key.clone(),
        )
        .map_err(|error| PluginError::Apply {
            plugin: self.name(),
            message: error.0,
        })?;
        Ok(vec![ctx.register(KV_STORE, std::sync::Arc::new(store))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_endpoint_and_encodes_keys() {
        assert!(ElasticsearchKVStore::new("redis://bad", "ah", None).is_err());
        assert_eq!(encode_id("a/b"), "612f62");
    }

    #[test]
    fn rest_roundtrip_uses_document_and_prefix_search_apis() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 2048];
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
                let text = String::from_utf8_lossy(&request);
                let body = if text.starts_with("GET ") {
                    r#"{"_source":{"key":"es:key","value":{"ok":true},"updated_ms":1}}"#
                } else if text.starts_with("POST ") {
                    r#"{"hits":{"hits":[{"_source":{"key":"es:key","value":{"ok":true},"updated_ms":1}}]}}"#
                } else {
                    "{}"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let store = ElasticsearchKVStore::new(
            format!("http://{address}"),
            "agent_harness",
            Some("test-token".into()),
        )
        .expect("client");
        store.set("es:key", json!({"ok": true})).expect("set");
        assert_eq!(store.get("es:key").expect("get"), Some(json!({"ok": true})));
        assert_eq!(store.scan("es:").expect("scan").len(), 1);
        store.delete("es:key").expect("delete");
        handle.join().expect("server");
    }
}
