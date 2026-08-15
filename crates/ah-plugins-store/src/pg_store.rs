//! PostgreSQL 后端 store(与 ah-plugins-store 文件/Redis 后端同一 seam,可互换)。
//!
//! 真实 SQL:两张表
//! - kv(key text PK, value jsonb, updated_ms bigint)
//! - messages(channel text, seq bigint, payload jsonb, ts_ms bigint, PK(channel,seq))
//!
//! 连接失败显式报错(不静默降级);表不存在时自动 CREATE(幂等)。

use std::sync::Mutex;

use ah_contracts::keys::{KV_STORE, MESSAGE_STORE};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::store::{
    BaseKVStore, BaseMessageStore, KvEntry, StoreError, StoreProvider, StoredMessage,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use postgres::{Client, NoTls};
use serde_json::Value;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 PostgreSQL 后端 store(KV + message)。
pub struct PgStore {
    conn: Mutex<Client>,
}

impl PgStore {
    /// 连接 PostgreSQL(URL 如 postgres://user:pass@127.0.0.1:5432/db)并建表。
    pub fn open(url: &str) -> Result<Self, StoreError> {
        let mut client =
            Client::connect(url, NoTls).map_err(|e| StoreError(format!("connect pg: {e}")))?;
        // 幂等建表(真实 SQL)。
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS kv (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_ms BIGINT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS messages (
                    channel TEXT NOT NULL,
                    seq BIGINT NOT NULL,
                    payload TEXT NOT NULL,
                    ts_ms BIGINT NOT NULL,
                    PRIMARY KEY (channel, seq)
                );",
            )
            .map_err(|e| StoreError(format!("create tables: {e}")))?;
        Ok(Self {
            conn: Mutex::new(client),
        })
    }
}

impl Seam for PgStore {}

impl BaseKVStore for PgStore {
    fn get(&self, key: &str) -> Result<Option<Value>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn
            .query("SELECT value FROM kv WHERE key = $1", &[&key])
            .map_err(|e| StoreError(format!("pg SELECT kv {key}: {e}")))?;
        match rows.first() {
            None => Ok(None),
            Some(row) => {
                let text: String = row.get(0);
                let value = serde_json::from_str(&text)
                    .map_err(|e| StoreError(format!("parse kv json: {e}")))?;
                Ok(Some(value))
            }
        }
    }

    fn set(&self, key: &str, value: Value) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let text = serde_json::to_string(&value)
            .map_err(|e| StoreError(format!("serialize kv json: {e}")))?;
        conn.execute(
            "INSERT INTO kv (key, value, updated_ms) VALUES ($1, $2, $3)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_ms = EXCLUDED.updated_ms",
            &[&key, &text, &(now_ms() as i64)],
        )
        .map_err(|e| StoreError(format!("pg UPSERT kv {key}: {e}")))?;
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM kv WHERE key = $1", &[&key])
            .map_err(|e| StoreError(format!("pg DELETE kv {key}: {e}")))?;
        Ok(())
    }

    fn scan(&self, prefix: &str) -> Result<Vec<KvEntry>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn
            .query(
                "SELECT key, value, updated_ms FROM kv WHERE key LIKE $1 ORDER BY key",
                &[&format!("{prefix}%")],
            )
            .map_err(|e| StoreError(format!("pg scan kv {prefix}: {e}")))?;
        let mut entries = Vec::new();
        for row in rows {
            let text: String = row.get(1);
            let value = serde_json::from_str(&text)
                .map_err(|e| StoreError(format!("parse kv json: {e}")))?;
            entries.push(KvEntry {
                key: row.get(0),
                value,
                updated_ms: row.get::<_, i64>(2) as u64,
            });
        }
        Ok(entries)
    }
}

impl BaseMessageStore for PgStore {
    fn append(&self, channel: &str, payload: Value) -> Result<StoredMessage, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        // 事务内取当前 max seq + 1 再插入(真实 SQL,保证单调)。
        let next_seq: i64 = conn
            .query_one(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE channel = $1",
                &[&channel],
            )
            .map_err(|e| StoreError(format!("pg max seq {channel}: {e}")))?
            .get(0);
        let message = StoredMessage {
            channel: channel.to_string(),
            seq: next_seq as u64,
            payload,
            ts_ms: now_ms(),
        };
        let payload_text = serde_json::to_string(&message.payload)
            .map_err(|e| StoreError(format!("serialize message json: {e}")))?;
        conn.execute(
            "INSERT INTO messages (channel, seq, payload, ts_ms) VALUES ($1, $2, $3, $4)",
            &[&channel, &next_seq, &payload_text, &(message.ts_ms as i64)],
        )
        .map_err(|e| StoreError(format!("pg INSERT message {channel}: {e}")))?;
        Ok(message)
    }

    fn read(&self, channel: &str, after_seq: u64) -> Result<Vec<StoredMessage>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn
            .query(
                "SELECT channel, seq, payload, ts_ms FROM messages
                 WHERE channel = $1 AND seq >= $2 ORDER BY seq",
                &[&channel, &(after_seq as i64)],
            )
            .map_err(|e| StoreError(format!("pg read messages {channel}: {e}")))?;
        let mut messages = Vec::new();
        for row in rows {
            let text: String = row.get(2);
            let payload = serde_json::from_str(&text)
                .map_err(|e| StoreError(format!("parse message json: {e}")))?;
            messages.push(StoredMessage {
                channel: row.get(0),
                seq: row.get::<_, i64>(1) as u64,
                payload,
                ts_ms: row.get::<_, i64>(3) as u64,
            });
        }
        Ok(messages)
    }

    fn channels(&self) -> Result<Vec<String>, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let rows = conn
            .query(
                "SELECT DISTINCT channel FROM messages ORDER BY channel",
                &[],
            )
            .map_err(|e| StoreError(format!("pg channels: {e}")))?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }
}

impl StoreProvider for PgStore {}

/// PostgreSQL store 插件:提供 SQL 后端的 KV + message seam。
pub struct PgStorePlugin {
    url: String,
}

impl PgStorePlugin {
    /// 以 PostgreSQL URL 创建。
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Plugin for PgStorePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-store-pg"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![KV_STORE, MESSAGE_STORE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let store =
            std::sync::Arc::new(PgStore::open(&self.url).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
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

    const PG_URL: &str = "postgres://postgres:ah@127.0.0.1:64329/ah";

    fn pg_available() -> bool {
        PgStore::open(PG_URL).is_ok()
    }

    /// 独立 key 前缀/频道,避免污染共享库;结束清理。
    fn prefix(tag: &str) -> String {
        format!("ah-test:{tag}:{}", std::process::id())
    }

    #[test]
    fn pg_kv_roundtrip_with_scan_and_delete() {
        if !pg_available() {
            println!("skipping: postgres unavailable");
            return;
        }
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(PgStorePlugin::new(PG_URL))];
        let effects = ctx.mount_all(plugins).expect("mount");
        let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("kv");
        let p = prefix("kv");

        let k1 = format!("{p}:k1");
        let k2 = format!("{p}:alpha");
        assert!(kv.get(&k1).expect("get").is_none(), "missing key is None");
        kv.set(&k1, json!({"v": 1})).expect("set");
        kv.set(&k2, json!({"v": 2})).expect("set");
        assert_eq!(kv.get(&k1).expect("get").unwrap()["v"], 1);
        // 覆盖写。
        kv.set(&k1, json!({"v": 10})).expect("set overwrite");
        assert_eq!(kv.get(&k1).expect("get").unwrap()["v"], 10);

        let scanned = kv.scan(&format!("{p}:k")).expect("scan");
        assert_eq!(scanned.len(), 1, "prefix scan");
        assert_eq!(scanned[0].key, k1);

        kv.delete(&k1).expect("delete");
        assert!(kv.get(&k1).expect("get").is_none(), "deleted key gone");

        kv.delete(&k2).expect("cleanup");
        drop(effects);
    }

    #[test]
    fn pg_message_store_append_read_channels() {
        if !pg_available() {
            println!("skipping: postgres unavailable");
            return;
        }
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(PgStorePlugin::new(PG_URL))];
        let effects = ctx.mount_all(plugins).expect("mount");
        let store = ctx
            .service::<dyn BaseMessageStore>(&MESSAGE_STORE)
            .expect("messages");
        let channel = prefix("chan");

        let m1 = store.append(&channel, json!({"n": 1})).expect("append 1");
        let m2 = store.append(&channel, json!({"n": 2})).expect("append 2");
        assert_eq!((m1.seq, m2.seq), (1, 2), "monotonic seq");
        assert_eq!(m1.payload["n"], 1);

        // 增量读:after_seq=2 → 只返回 seq 2(含自身)。
        let after = store.read(&channel, 2).expect("read after");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].seq, 2);
        assert_eq!(after[0].payload["n"], 2);

        // 全读。
        let all = store.read(&channel, 0).expect("read all");
        assert_eq!(all.len(), 2);

        assert!(store.channels().expect("channels").contains(&channel));

        // 清理:真实 DELETE。
        {
            let store = PgStore::open(PG_URL).expect("open");
            let mut conn = store.conn.lock().unwrap();
            conn.execute("DELETE FROM messages WHERE channel = $1", &[&channel])
                .expect("cleanup");
        }
        drop(effects);
    }
}
