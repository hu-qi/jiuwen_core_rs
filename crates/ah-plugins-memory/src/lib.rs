//! # ah-plugins-memory
//!
//! 真实持久化记忆:每条记忆一个 JSON 文件,store 时落盘、启动时恢复。
//! 提供 memory seam + remember/recall/forget 三个真实工具。

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::{KV_STORE, MEMORY, TOOLS};
use ah_contracts::memory::{MemoryError, MemoryProvider, MemoryRecord};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::store::BaseKVStore;
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

/// 真实 JSON 文件记忆 provider:dir/{key}.json。
pub struct JsonFileMemoryProvider {
    dir: PathBuf,
    /// 内存索引(key -> record),启动时从磁盘加载。
    index: Mutex<std::collections::HashMap<String, MemoryRecord>>,
}

impl JsonFileMemoryProvider {
    /// 以记忆目录创建 provider;目录不存在则创建;加载已有记忆。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| MemoryError(format!("create memory dir failed: {e}")))?;
        let mut index = std::collections::HashMap::new();
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| MemoryError(format!("read memory dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| MemoryError(format!("entry failed: {e}")))?;
            if entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("json")
                && let Ok(text) = std::fs::read_to_string(entry.path())
                && let Ok(record) = serde_json::from_str::<MemoryRecord>(&text)
            {
                index.insert(record.key.clone(), record);
            }
        }
        Ok(Self {
            dir,
            index: Mutex::new(index),
        })
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.json", encode_memory_key(key)))
    }
}

fn encode_memory_key(key: &str) -> String {
    if !key.is_empty()
        && !key.starts_with("x-")
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return key.to_string();
    }
    let mut encoded = String::from("x-");
    for byte in key.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

impl Seam for JsonFileMemoryProvider {}

impl MemoryProvider for JsonFileMemoryProvider {
    fn store(
        &self,
        key: &str,
        content: &str,
        tags: Vec<String>,
    ) -> Result<MemoryRecord, MemoryError> {
        if key.trim().is_empty() {
            return Err(MemoryError("memory key must not be empty".to_string()));
        }
        let record = MemoryRecord {
            key: key.to_string(),
            content: content.to_string(),
            tags,
            created_ms: now_ms(),
        };
        let text = serde_json::to_string(&record)
            .map_err(|e| MemoryError(format!("serialize failed: {e}")))?;
        std::fs::write(self.path_for(key), text)
            .map_err(|e| MemoryError(format!("write failed: {e}")))?;
        self.index
            .lock()
            .unwrap()
            .insert(key.to_string(), record.clone());
        Ok(record)
    }

    fn retrieve(&self, key: &str) -> Option<MemoryRecord> {
        self.index.lock().unwrap().get(key).cloned()
    }

    fn search(&self, query: &str) -> Vec<MemoryRecord> {
        let query = query.to_lowercase();
        let mut results: Vec<MemoryRecord> = self
            .index
            .lock()
            .unwrap()
            .values()
            .filter(|record| {
                record.content.to_lowercase().contains(&query)
                    || record
                        .tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        results.sort_by(|a, b| a.key.cmp(&b.key));
        results
    }

    fn list(&self) -> Vec<MemoryRecord> {
        let mut records: Vec<MemoryRecord> = self.index.lock().unwrap().values().cloned().collect();
        records.sort_by(|a, b| a.key.cmp(&b.key));
        records
    }

    fn remove(&self, key: &str) -> Result<(), MemoryError> {
        let path = self.path_for(key);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| MemoryError(format!("remove failed: {e}")))?;
        }
        self.index.lock().unwrap().remove(key);
        Ok(())
    }
}

/// 基于 KV seam 的外部记忆 provider。
///
/// 生产 profile 中 KV seam 可由 Redis 等真实后端提供；记忆记录仍使用稳定
/// JSON 格式，因而可在不改变 memory seam 的情况下切换存储实现。
pub struct KeyValueMemoryProvider {
    store: std::sync::Arc<dyn BaseKVStore>,
}

impl KeyValueMemoryProvider {
    pub fn new(store: std::sync::Arc<dyn BaseKVStore>) -> Self {
        Self { store }
    }

    fn storage_key(key: &str) -> String {
        format!("memory:{}", encode_memory_key(key))
    }

    fn decode(entry: &serde_json::Value) -> Option<MemoryRecord> {
        serde_json::from_value(entry.clone()).ok()
    }
}

impl Seam for KeyValueMemoryProvider {}

impl MemoryProvider for KeyValueMemoryProvider {
    fn store(
        &self,
        key: &str,
        content: &str,
        tags: Vec<String>,
    ) -> Result<MemoryRecord, MemoryError> {
        if key.trim().is_empty() {
            return Err(MemoryError("memory key must not be empty".to_string()));
        }
        let record = MemoryRecord {
            key: key.to_string(),
            content: content.to_string(),
            tags,
            created_ms: now_ms(),
        };
        let value = serde_json::to_value(&record)
            .map_err(|e| MemoryError(format!("serialize failed: {e}")))?;
        self.store
            .set(&Self::storage_key(key), value)
            .map_err(|e| MemoryError(format!("external memory write failed: {e}")))?;
        Ok(record)
    }

    fn retrieve(&self, key: &str) -> Option<MemoryRecord> {
        self.store
            .get(&Self::storage_key(key))
            .ok()
            .flatten()
            .and_then(|value| Self::decode(&value))
    }

    fn search(&self, query: &str) -> Vec<MemoryRecord> {
        let query = query.to_lowercase();
        let mut records: Vec<_> = self
            .store
            .scan("memory:")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::decode(&entry.value))
            .filter(|record| {
                record.content.to_lowercase().contains(&query)
                    || record
                        .tags
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&query))
            })
            .collect();
        records.sort_by(|a, b| a.key.cmp(&b.key));
        records
    }

    fn list(&self) -> Vec<MemoryRecord> {
        let mut records: Vec<_> = self
            .store
            .scan("memory:")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::decode(&entry.value))
            .collect();
        records.sort_by(|a, b| a.key.cmp(&b.key));
        records
    }

    fn remove(&self, key: &str) -> Result<(), MemoryError> {
        self.store
            .delete(&Self::storage_key(key))
            .map_err(|e| MemoryError(format!("external memory delete failed: {e}")))
    }
}

/// remember 工具:存储一条记忆。
pub struct RememberTool {
    memory: std::sync::Arc<dyn MemoryProvider>,
}

impl RememberTool {
    pub fn new(memory: std::sync::Arc<dyn MemoryProvider>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for RememberTool {
    fn name(&self) -> &'static str {
        "remember"
    }

    fn description(&self) -> &'static str {
        "store a memory; arguments: {key, content, tags?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string" },
                "content": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } },
            },
            "required": ["key", "content"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let key = arguments
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field key".to_string()))?;
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field content".to_string()))?;
        let tags: Vec<String> = arguments
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let record = self
            .memory
            .store(key, content, tags)
            .map_err(|e| ToolError(format!("memory store failed: {e}")))?;
        Ok(json!({ "key": record.key, "stored": true }))
    }
}

/// recall 工具:搜索记忆。
pub struct RecallTool {
    memory: std::sync::Arc<dyn MemoryProvider>,
}

impl RecallTool {
    pub fn new(memory: std::sync::Arc<dyn MemoryProvider>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for RecallTool {
    fn name(&self) -> &'static str {
        "recall"
    }

    fn description(&self) -> &'static str {
        "search memories; arguments: {query}"
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
        let records = self.memory.search(query);
        Ok(json!({ "count": records.len(), "records": records }))
    }
}

/// forget 工具:删除一条记忆。
pub struct ForgetTool {
    memory: std::sync::Arc<dyn MemoryProvider>,
}

impl ForgetTool {
    pub fn new(memory: std::sync::Arc<dyn MemoryProvider>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ForgetTool {
    fn name(&self) -> &'static str {
        "forget"
    }

    fn description(&self) -> &'static str {
        "remove a memory; arguments: {key}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "key": { "type": "string" } },
            "required": ["key"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let key = arguments
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field key".to_string()))?;
        self.memory
            .remove(key)
            .map_err(|e| ToolError(format!("memory remove failed: {e}")))?;
        Ok(json!({ "key": key, "removed": true }))
    }
}

/// 记忆插件:提供 memory seam,并注册 remember/recall/forget 工具。
pub struct MemoryPlugin {
    dir: PathBuf,
}

impl MemoryPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for MemoryPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-memory"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MEMORY]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: std::sync::Arc<dyn MemoryProvider> =
            if let Some(store) = ctx.service::<dyn BaseKVStore>(&KV_STORE) {
                std::sync::Arc::new(KeyValueMemoryProvider::new(store))
            } else {
                std::sync::Arc::new(JsonFileMemoryProvider::open(&self.dir).map_err(|e| {
                    PluginError::Apply {
                        plugin: self.name(),
                        message: e.0,
                    }
                })?)
            };
        let mut effects = vec![ctx.register(MEMORY, provider.clone())];

        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(std::sync::Arc::new(RememberTool::new(provider.clone()))));
        effects.push(registry.register(std::sync::Arc::new(RecallTool::new(provider.clone()))));
        effects.push(registry.register(std::sync::Arc::new(ForgetTool::new(provider))));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(tag: &str) -> (JsonFileMemoryProvider, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ah-memory-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let provider = JsonFileMemoryProvider::open(&dir).expect("open");
        (provider, dir)
    }
    #[test]
    fn key_value_memory_provider_roundtrips_and_searches() {
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
        let memory = KeyValueMemoryProvider::new(store);
        let record = memory
            .store("user/name", "Alice", vec!["person".into()])
            .expect("store");
        assert_eq!(memory.retrieve("user/name"), Some(record));
        assert_eq!(memory.search("alice").len(), 1);
        assert_eq!(memory.list().len(), 1);
        memory.remove("user/name").expect("remove");
        assert!(memory.retrieve("user/name").is_none());
    }

    #[test]
    fn store_retrieve_search_remove_roundtrip() {
        let (memory, dir) = provider("roundtrip");
        memory
            .store("user-name", "Alice", vec!["user".into()])
            .expect("store");
        assert_eq!(memory.retrieve("user-name").unwrap().content, "Alice");

        // 搜索:内容与标签。
        assert_eq!(memory.search("alice").len(), 1);
        assert_eq!(memory.search("user").len(), 1);
        assert_eq!(memory.search("nope").len(), 0);

        memory.remove("user-name").expect("remove");
        assert!(memory.retrieve("user-name").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_across_reopen() {
        let (memory, dir) = provider("persist");
        memory
            .store("pref-language", "Rust", vec![])
            .expect("store");
        drop(memory);

        // 重新打开:真实文件恢复。
        let reopened = JsonFileMemoryProvider::open(&dir).expect("reopen");
        assert_eq!(reopened.retrieve("pref-language").unwrap().content, "Rust");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn memory_plugin_registers_tools() {
        use ah_contracts::keys::TOOLS;
        use ah_hub::plugin::DynPlugin;
        use std::sync::Arc as StdArc;

        let dir = std::env::temp_dir().join(format!("ah-memory-plugin-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(MemoryPlugin::new(&dir)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");

        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let mut names = registry.names();
        names.sort();
        assert_eq!(names, vec!["forget", "recall", "remember"]);

        // 工具真实调用:remember -> recall 往返(真实文件落盘)。
        let _ = registry
            .invoke(
                "remember",
                json!({ "key": "k", "content": "hello memory", "tags": ["greeting"] }),
            )
            .await
            .expect("remember");
        let result = registry
            .invoke("recall", json!({ "query": "hello" }))
            .await
            .expect("recall");
        assert_eq!(result["count"], 1);
        assert!(dir.join("k.json").exists(), "记忆真实落盘");

        drop(effects);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn memory_keys_cannot_escape_directory_or_be_empty() {
        let (memory, dir) = provider("key-boundary");
        memory
            .store("../escape", "safe content", Vec::new())
            .expect("store encoded key");
        assert!(!dir.parent().unwrap().join("escape.json").exists());
        drop(memory);
        let reopened = JsonFileMemoryProvider::open(&dir).expect("reopen encoded key");
        assert_eq!(
            reopened.retrieve("../escape").unwrap().content,
            "safe content"
        );
        assert!(reopened.store(" ", "invalid", Vec::new()).is_err());
        reopened.remove("../escape").expect("remove encoded key");
        assert!(reopened.retrieve("../escape").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn redis_memory_provider_roundtrips_searches_and_removes() {
        use ah_contracts::store::BaseKVStore;
        use std::sync::Arc;

        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into());
        let store = match ah_plugins_store::RedisKVStore::open(&redis_url) {
            Ok(store) => Arc::new(store),
            Err(error) => {
                println!("skipping: redis unavailable: {error}");
                return;
            }
        };
        let memory = KeyValueMemoryProvider::new(store.clone());
        let key = format!("external-memory-{}", std::process::id());
        let storage_key = KeyValueMemoryProvider::storage_key(&key);
        let _ = store.delete(&storage_key);

        let record = memory
            .store(&key, "Redis-backed memory", vec!["external".into()])
            .expect("store");
        assert_eq!(memory.retrieve(&key), Some(record.clone()));
        assert_eq!(memory.search("redis-backed").len(), 1);
        assert_eq!(memory.search("external").len(), 1);
        assert!(memory.list().iter().any(|item| item.key == key));

        memory.remove(&key).expect("remove");
        assert!(memory.retrieve(&key).is_none());
        assert!(store.get(&storage_key).expect("get cleanup").is_none());
    }
}
