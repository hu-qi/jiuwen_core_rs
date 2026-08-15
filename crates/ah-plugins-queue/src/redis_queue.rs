//! Redis 后端 MessageQueue(与 ah-plugins-queue 文件后端同一 seam,可互换)。
//!
//! 与文件后端同构:日志为真相 + 游标。每 channel:
//! - 日志:Redis LIST(RPUSH 追加消息 JSON);
//! - 序号:Redis 计数器(INCR 生成单调 seq);
//! - 游标:Redis STRING(已消费最大 seq,offset 语义)。
//!
//! 连接失败显式报错(不静默降级)。

use std::sync::Mutex;

use ah_contracts::keys::QUEUE;
use ah_contracts::prelude::Effect;
use ah_contracts::queue::{MessageQueue, QueueError, QueueMessage};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
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

/// 真实 Redis 后端队列。
pub struct RedisQueue {
    conn: Mutex<Connection>,
    /// 隔离前缀(多租户/测试避免互相污染)。
    prefix: String,
}

impl RedisQueue {
    /// 连接 Redis(URL 如 redis://127.0.0.1:6379/)。
    pub fn open(url: &str) -> Result<Self, QueueError> {
        Self::open_with_prefix(url, "ah:q")
    }

    /// 连接 Redis 并使用自定义 key 前缀。
    pub fn open_with_prefix(url: &str, prefix: &str) -> Result<Self, QueueError> {
        let client = Client::open(url).map_err(|e| QueueError(format!("open redis: {e}")))?;
        let conn = client
            .get_connection()
            .map_err(|e| QueueError(format!("connect redis: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
            prefix: prefix.to_string(),
        })
    }

    fn log_key(&self, channel: &str) -> String {
        format!("{}:{channel}:log", self.prefix)
    }

    fn seq_key(&self, channel: &str) -> String {
        format!("{}:{channel}:seq", self.prefix)
    }

    fn cursor_key(&self, channel: &str) -> String {
        format!("{}:{channel}:cursor", self.prefix)
    }
}

impl Seam for RedisQueue {}

impl MessageQueue for RedisQueue {
    fn publish(&self, channel: &str, payload: Value) -> Result<QueueMessage, QueueError> {
        let mut conn = self.conn.lock().unwrap();
        // 原子递增序号(INCR 保证单调)。
        let seq: u64 = conn
            .incr(self.seq_key(channel), 1)
            .map_err(|e| QueueError(format!("redis INCR {}: {e}", self.seq_key(channel))))?;
        let message = QueueMessage {
            channel: channel.to_string(),
            seq,
            payload,
            ts_ms: now_ms(),
        };
        let line = serde_json::to_string(&message)
            .map_err(|e| QueueError(format!("serialize message: {e}")))?;
        // 日志为真相:追加到 LIST 尾部。
        conn.rpush::<_, _, ()>(self.log_key(channel), line)
            .map_err(|e| QueueError(format!("redis RPUSH {}: {e}", self.log_key(channel))))?;
        Ok(message)
    }

    fn consume(&self, channel: &str) -> Result<Option<QueueMessage>, QueueError> {
        let mut conn = self.conn.lock().unwrap();
        // 游标 = 下一个待消费的 seq(offset 语义)。
        let raw_cursor: Option<u64> = conn.get(self.cursor_key(channel)).map_err(|e| {
            QueueError(format!(
                "redis GET cursor {}: {e}",
                self.cursor_key(channel)
            ))
        })?;
        let cursor = raw_cursor.unwrap_or(0);
        // 从日志按索引取第 cursor 条(0 起)。
        let index = cursor as isize;
        let raw: Option<String> = conn
            .lindex(self.log_key(channel), index)
            .map_err(|e| QueueError(format!("redis LINDEX {}: {e}", self.log_key(channel))))?;
        let Some(line) = raw else {
            return Ok(None);
        };
        let message: QueueMessage =
            serde_json::from_str(&line).map_err(|e| QueueError(format!("parse message: {e}")))?;
        // 推进游标。
        conn.set::<_, _, ()>(self.cursor_key(channel), cursor + 1)
            .map_err(|e| {
                QueueError(format!(
                    "redis SET cursor {}: {e}",
                    self.cursor_key(channel)
                ))
            })?;
        Ok(Some(message))
    }

    fn backlog(&self, channel: &str) -> Result<Vec<QueueMessage>, QueueError> {
        let mut conn = self.conn.lock().unwrap();
        let lines: Vec<String> = conn
            .lrange(self.log_key(channel), 0, -1)
            .map_err(|e| QueueError(format!("redis LRANGE {}: {e}", self.log_key(channel))))?;
        lines
            .iter()
            .map(|line| {
                serde_json::from_str::<QueueMessage>(line)
                    .map_err(|e| QueueError(format!("parse message: {e}")))
            })
            .collect()
    }

    fn channels(&self) -> Result<Vec<String>, QueueError> {
        let mut conn = self.conn.lock().unwrap();
        let pattern = format!("{}:*:log", self.prefix);
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pattern)
            .query(&mut *conn)
            .map_err(|e| QueueError(format!("redis KEYS {pattern}: {e}")))?;
        // ah:q:<channel>:log → <channel>(去前缀去后缀,去重)。
        let mut channels: Vec<String> = keys
            .iter()
            .filter_map(|key| {
                let stripped = key
                    .strip_prefix(&format!("{}:", self.prefix))?
                    .strip_suffix(":log")?;
                Some(stripped.to_string())
            })
            .collect();
        channels.sort();
        channels.dedup();
        Ok(channels)
    }
}

/// Redis 队列插件:提供 Redis 后端的 queue seam。
pub struct RedisQueuePlugin {
    url: String,
    prefix: String,
}

impl RedisQueuePlugin {
    /// 以 Redis URL 创建(默认本地 6379)。
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            prefix: "ah:q".to_string(),
        }
    }
}

impl Plugin for RedisQueuePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-queue-redis"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![QUEUE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let queue = RedisQueue::open_with_prefix(&self.url, &self.prefix).map_err(|e| {
            PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            }
        })?;
        let queue: std::sync::Arc<dyn MessageQueue> = std::sync::Arc::new(queue);
        Ok(vec![ctx.register(QUEUE, queue)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::QUEUE;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc as StdArc;

    const REDIS_URL: &str = "redis://127.0.0.1:6379/";

    fn redis_available() -> bool {
        RedisQueue::open(REDIS_URL).is_ok()
    }

    /// 独立前缀,避免污染共享 Redis;结束清理。
    fn prefix(tag: &str) -> String {
        format!("ah-test:q:{tag}:{}", std::process::id())
    }

    #[test]
    fn redis_queue_publish_consume_backlog_and_channels() {
        if !redis_available() {
            println!("skipping: redis unavailable");
            return;
        }
        let p = prefix("round");
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(RedisQueuePlugin {
            url: REDIS_URL.to_string(),
            prefix: p.clone(),
        })];
        let effects = ctx.mount_all(plugins).expect("mount");
        let queue = ctx.service::<dyn MessageQueue>(&QUEUE).expect("queue");

        // 发布三条。
        let m1 = queue.publish("news", json!({"n": 1})).expect("publish 1");
        let m2 = queue.publish("news", json!({"n": 2})).expect("publish 2");
        let m3 = queue.publish("news", json!({"n": 3})).expect("publish 3");
        assert_eq!((m1.seq, m2.seq, m3.seq), (1, 2, 3), "monotonic seq");

        // 消费按序推进游标。
        let c1 = queue.consume("news").expect("consume 1").expect("some");
        let c2 = queue.consume("news").expect("consume 2").expect("some");
        assert_eq!(c1.seq, 1);
        assert_eq!(c2.seq, 2);
        assert_eq!(c1.payload["n"], 1);

        // backlog 保留全部消息(日志为真相)。
        let backlog = queue.backlog("news").expect("backlog");
        assert_eq!(backlog.len(), 3);

        // 游标持久:重开连接后从 seq 3 继续。
        drop(effects);
        let ctx2 = Context::new();
        let plugins2: Vec<DynPlugin> = vec![StdArc::new(RedisQueuePlugin {
            url: REDIS_URL.to_string(),
            prefix: p.clone(),
        })];
        let effects2 = ctx2.mount_all(plugins2).expect("mount");
        let queue2 = ctx2.service::<dyn MessageQueue>(&QUEUE).expect("queue");
        let c3 = queue2.consume("news").expect("consume 3").expect("some");
        assert_eq!(c3.seq, 3, "cursor persisted across reopen");

        // channels 列出有日志的 channel。
        assert!(
            queue2
                .channels()
                .expect("channels")
                .contains(&"news".to_string())
        );
        assert!(queue2.consume("empty").expect("empty").is_none());

        // 清理:删掉本测试的 keys。
        let mut conn = redis::Client::open(REDIS_URL)
            .expect("open")
            .get_connection()
            .expect("connect");
        for key in [
            format!("{p}:news:log"),
            format!("{p}:news:seq"),
            format!("{p}:news:cursor"),
        ] {
            conn.del::<_, ()>(&key).expect("cleanup");
        }
        drop(effects2);
    }
}
