//! # ah-plugins-store
//!
//! 真实文件后端 store(对应 openjiuwen/core 的 BaseKVStore / BaseMessageStore):
//!
//! - KV:dir/{key}.json,set 落盘、get/delete 真实读写,scan 按前缀枚举;
//! - message:dir/messages/{channel}.jsonl append-only,read 按 seq 增量。
//!
//! - 外部后端:Redis KV 与 PostgreSQL KV/message 由同 crate 的真实插件提供。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::{KV_STORE, MESSAGE_STORE};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::store::{
    BaseKVStore, BaseMessageStore, KvEntry, StoreError, StoreProvider, StoredMessage,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

pub mod elasticsearch;
pub mod pg_store;
pub mod redis_store;
pub use elasticsearch::{ElasticsearchKVStore, ElasticsearchStorePlugin};
pub use pg_store::{GaussDbStore, GaussDbStorePlugin, PgStore, PgStorePlugin};
pub use redis_store::{RedisKVStore, RedisStorePlugin};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实文件后端 store。
pub struct FileStore {
    dir: PathBuf,
    /// channel → 下一序号。
    next_seq: Mutex<std::collections::HashMap<String, u64>>,
}

impl FileStore {
    /// 以根目录创建(自动创建 kv/ 与 messages/ 子目录)。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        std::fs::create_dir_all(dir.join("kv"))
            .map_err(|e| StoreError(format!("create kv dir failed: {e}")))?;
        std::fs::create_dir_all(dir.join("messages"))
            .map_err(|e| StoreError(format!("create messages dir failed: {e}")))?;
        Ok(Self {
            dir,
            next_seq: Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn kv_path(&self, key: &str) -> PathBuf {
        self.dir.join("kv").join(format!("{key}.json"))
    }

    fn channel_path(&self, channel: &str) -> PathBuf {
        self.dir.join("messages").join(format!("{channel}.jsonl"))
    }
}

impl Seam for FileStore {}

impl BaseKVStore for FileStore {
    fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
        let path = self.kv_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| StoreError(format!("read kv: {e}")))?;
        let entry: KvEntry =
            serde_json::from_str(&text).map_err(|e| StoreError(format!("parse kv: {e}")))?;
        Ok(Some(entry.value))
    }

    fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
        let entry = KvEntry {
            key: key.to_string(),
            value,
            updated_ms: now_ms(),
        };
        let line =
            serde_json::to_string(&entry).map_err(|e| StoreError(format!("serialize kv: {e}")))?;
        std::fs::write(self.kv_path(key), line).map_err(|e| StoreError(format!("write kv: {e}")))
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        let path = self.kv_path(key);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| StoreError(format!("delete kv: {e}")))?;
        }
        Ok(())
    }

    fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(self.dir.join("kv"))
            .map_err(|e| StoreError(format!("read kv dir: {e}")))?
        {
            let entry = entry.map_err(|e| StoreError(format!("kv entry: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(key) = name.strip_suffix(".json") else {
                continue;
            };
            if !key.starts_with(prefix) {
                continue;
            }
            let text = std::fs::read_to_string(entry.path())
                .map_err(|e| StoreError(format!("read kv: {e}")))?;
            let parsed: KvEntry =
                serde_json::from_str(&text).map_err(|e| StoreError(format!("parse kv: {e}")))?;
            entries.push(parsed);
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(entries)
    }
}

impl BaseMessageStore for FileStore {
    fn append(&self, channel: &str, payload: Value) -> Result<StoredMessage, StoreError> {
        let mut seqs = self.next_seq.lock().unwrap();
        let seq = seqs.entry(channel.to_string()).or_insert(0);
        let message = StoredMessage {
            channel: channel.to_string(),
            seq: *seq,
            payload,
            ts_ms: now_ms(),
        };
        *seq += 1;
        drop(seqs);

        let line = serde_json::to_string(&message)
            .map_err(|e| StoreError(format!("serialize message: {e}")))?;
        let path = self.channel_path(channel);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StoreError(format!("open channel: {e}")))?;
        use std::io::Write;
        writeln!(file, "{line}").map_err(|e| StoreError(format!("append message: {e}")))?;
        Ok(message)
    }

    fn read(&self, channel: &str, after_seq: u64) -> Result<Vec<StoredMessage>, StoreError> {
        let path = self.channel_path(channel);
        if !path.exists() {
            return Ok(vec![]);
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| StoreError(format!("read channel: {e}")))?;
        let mut messages = Vec::new();
        for line in text.lines() {
            let message: StoredMessage = serde_json::from_str(line)
                .map_err(|e| StoreError(format!("parse message: {e}")))?;
            if message.seq >= after_seq {
                messages.push(message);
            }
        }
        Ok(messages)
    }

    fn channels(&self) -> Result<Vec<String>, StoreError> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(self.dir.join("messages"))
            .map_err(|e| StoreError(format!("read messages dir: {e}")))?
        {
            let entry = entry.map_err(|e| StoreError(format!("entry: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(channel) = name.strip_suffix(".jsonl") {
                names.push(channel.to_string());
            }
        }
        names.sort();
        Ok(names)
    }
}

impl StoreProvider for FileStore {}

/// store 插件:提供 KV 与 message 两种文件后端。
pub struct StorePlugin {
    dir: PathBuf,
}

impl StorePlugin {
    /// 以根目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for StorePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-store"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![KV_STORE, MESSAGE_STORE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let store = std::sync::Arc::new(FileStore::open(self.dir.clone()).map_err(|e| {
            PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            }
        })?);
        let kv: std::sync::Arc<dyn BaseKVStore> = store.clone();
        let messages: std::sync::Arc<dyn BaseMessageStore> = store;
        Ok(vec![
            ctx.register(KV_STORE, kv),
            ctx.register(MESSAGE_STORE, messages),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{KV_STORE, MESSAGE_STORE};
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(StorePlugin::new(root.join("store")))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn kv_roundtrip_with_scan_and_delete() {
        let root = std::env::temp_dir().join(format!("ah-store-kv-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("kv");

        assert!(kv.get("k1").expect("get").is_none(), "missing key is None");
        kv.set("k1", json!({"v": 1})).expect("set");
        kv.set("alpha", json!({"v": 2})).expect("set");
        assert_eq!(kv.get("k1").expect("get").unwrap()["v"], 1);
        let scanned = kv.scan("k").expect("scan");
        assert_eq!(scanned.len(), 1, "prefix scan k");
        assert_eq!(scanned[0].key, "k1");
        kv.delete("k1").expect("delete");
        assert!(kv.get("k1").expect("get").is_none(), "deleted key gone");
        kv.delete("k1").expect("delete again (idempotent)");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn message_append_read_incremental_and_channels() {
        let root = std::env::temp_dir().join(format!("ah-store-msg-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let store = ctx
            .service::<dyn BaseMessageStore>(&MESSAGE_STORE)
            .expect("messages");

        let m0 = store
            .append("room", json!({"from": "a", "text": "hi"}))
            .expect("m0");
        let m1 = store
            .append("room", json!({"from": "b", "text": "yo"}))
            .expect("m1");
        assert_eq!(m0.seq, 0);
        assert_eq!(m1.seq, 1);

        let after = store.read("room", 1).expect("read after");
        assert_eq!(after.len(), 1, "incremental read");
        assert_eq!(after[0].payload["text"], "yo");

        let all = store.read("room", 0).expect("read all");
        assert_eq!(all.len(), 2);

        assert!(
            store
                .channels()
                .expect("channels")
                .contains(&"room".to_string())
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-store-re-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("kv");
        kv.set("persist", json!({"ok": true})).expect("set");
        let msg = ctx
            .service::<dyn BaseMessageStore>(&MESSAGE_STORE)
            .expect("messages");
        msg.append("c1", json!({"n": 1})).expect("append");
        drop(effects);

        // 重开:KV 从磁盘读,message 从 JSONL 读。
        let reopened = FileStore::open(root.join("store")).expect("reopen");
        assert_eq!(reopened.get("persist").expect("get").unwrap()["ok"], true);
        let msgs = reopened.read("c1", 0).expect("read");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].payload["n"], 1);

        let _ = std::fs::remove_dir_all(&root);
    }
}
