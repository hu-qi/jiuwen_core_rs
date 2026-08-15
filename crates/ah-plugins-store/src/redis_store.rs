//! Redis 后端 BaseKVStore(与 ah-plugins-store 文件后端同一 seam,可互换)。
//!
//! 值格式与文件后端一致(KvEntry JSON),get/set/delete/scan 全部经真实
//! Redis 命令(SET/GET/DEL/KEYS);连接失败显式报错。

use std::sync::Mutex;

use ah_contracts::keys::KV_STORE;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::store::{BaseKVStore, KvEntry, StoreError};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use redis::{Client, Commands, Connection};
use serde_json::Value;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 Redis 后端 KV store。
pub struct RedisKVStore {
    conn: Mutex<Connection>,
}

impl RedisKVStore {
    /// 连接 Redis(URL 如 redis://127.0.0.1:6379/)。
    pub fn open(url: &str) -> Result<Self, StoreError> {
        let client = Client::open(url).map_err(|e| StoreError(format!("open redis: {e}")))?;
        let conn = client
            .get_connection()
            .map_err(|e| StoreError(format!("connect redis: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl Seam for RedisKVStore {}

impl BaseKVStore for RedisKVStore {
    fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let raw: Option<String> = conn
            .get(key)
            .map_err(|e| StoreError(format!("redis GET {key}: {e}")))?;
        match raw {
            None => Ok(None),
            Some(text) => {
                let entry: KvEntry = serde_json::from_str(&text)
                    .map_err(|e| StoreError(format!("parse kv: {e}")))?;
                Ok(Some(entry.value))
            }
        }
    }

    fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
        let entry = KvEntry {
            key: key.to_string(),
            value,
            updated_ms: now_ms(),
        };
        let line =
            serde_json::to_string(&entry).map_err(|e| StoreError(format!("serialize kv: {e}")))?;
        let mut conn = self.conn.lock().unwrap();
        conn.set::<_, _, ()>(key, line)
            .map_err(|e| StoreError(format!("redis SET {key}: {e}")))?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        conn.del::<_, ()>(key)
            .map_err(|e| StoreError(format!("redis DEL {key}: {e}")))?;
        Ok(())
    }

    fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let pattern = format!("{prefix}*");
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query(&mut *conn)
            .map_err(|e| StoreError(format!("redis KEYS {pattern}: {e}")))?;
        let mut entries = Vec::new();
        for key in keys {
            let raw: Option<String> = conn
                .get(&key)
                .map_err(|e| StoreError(format!("redis GET {key}: {e}")))?;
            if let Some(text) = raw
                && let Ok(entry) = serde_json::from_str::<KvEntry>(&text)
            {
                entries.push(entry);
            }
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(entries)
    }
}

/// Redis store 插件:提供 Redis 后端的 KV seam。
pub struct RedisStorePlugin {
    url: String,
}

impl RedisStorePlugin {
    /// 以 Redis URL 创建(默认本地 6379)。
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Plugin for RedisStorePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-store-redis"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![KV_STORE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let store = RedisKVStore::open(&self.url).map_err(|e| PluginError::Apply {
            plugin: self.name(),
            message: e.0,
        })?;
        let kv: std::sync::Arc<dyn BaseKVStore> = std::sync::Arc::new(store);
        Ok(vec![ctx.register(KV_STORE, kv)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::KV_STORE;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc as StdArc;

    const REDIS_URL: &str = "redis://127.0.0.1:6379/";

    fn redis_available() -> bool {
        RedisKVStore::open(REDIS_URL).is_ok()
    }

    /// 独立前缀,避免污染共享 Redis;结束清理。
    fn prefix(tag: &str) -> String {
        format!("ah-test:{tag}:{}", std::process::id())
    }

    #[test]
    fn redis_kv_roundtrip_with_scan_and_delete() {
        if !redis_available() {
            println!("skipping: redis unavailable");
            return;
        }
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(RedisStorePlugin::new(REDIS_URL))];
        let effects = ctx.mount_all(plugins).expect("mount");
        let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("kv");
        let p = prefix("round");

        let k1 = format!("{p}:k1");
        let k2 = format!("{p}:alpha");
        assert!(kv.get(&k1).expect("get").is_none(), "missing key is None");
        kv.set(&k1, json!({"v": 1})).expect("set");
        kv.set(&k2, json!({"v": 2})).expect("set");
        assert_eq!(kv.get(&k1).expect("get").unwrap()["v"], 1);

        let scanned = kv.scan(&format!("{p}:k")).expect("scan");
        assert_eq!(scanned.len(), 1, "prefix scan");
        assert_eq!(scanned[0].key, k1);

        kv.delete(&k1).expect("delete");
        assert!(kv.get(&k1).expect("get").is_none(), "deleted key gone");

        // 清理。
        kv.delete(&k2).expect("cleanup");
        drop(effects);
    }
}
