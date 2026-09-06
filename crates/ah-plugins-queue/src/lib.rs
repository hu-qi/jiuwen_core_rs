//! # ah-plugins-queue
//!
//! 真实文件后端消息队列(对应 openjiuwen messager/queue 的本地基础):
//! 每 channel 一个 append-only JSONL(日志为真相)+ 消费游标文件,
//! 与 Kafka 的 log+offset 模型同构(小规模)。外部传输
//! (Pulsar/ZMQ ROUTER-DEALER)留待后续,文档注明。
//! Redis 后端见 redis_queue 模块(同一 seam 可互换)。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::QUEUE;
use ah_contracts::prelude::Effect;
use ah_contracts::queue::{MessageQueue, QueueError, QueueMessage};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

pub mod pulsar;
pub mod redis_queue;
pub use pulsar::{PulsarRestQueue, PulsarRestQueuePlugin};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实文件后端队列。
pub struct FileQueue {
    dir: PathBuf,
    /// channel → 下一序号(启动时从日志恢复)。
    next_seq: Mutex<std::collections::HashMap<String, u64>>,
    /// channel → 已消费游标(已推进的最大 seq;启动时从游标文件恢复)。
    cursors: Mutex<std::collections::HashMap<String, u64>>,
}

impl FileQueue {
    /// 打开(或创建)队列目录;恢复日志与游标。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, QueueError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| QueueError(format!("create queue dir failed: {e}")))?;
        let mut next_seq = std::collections::HashMap::new();
        let mut cursors = std::collections::HashMap::new();
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| QueueError(format!("read queue dir failed: {e}")))?
        {
            let entry = entry.map_err(|e| QueueError(format!("entry failed: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(channel) = name.strip_suffix(".jsonl") {
                // 日志恢复:next_seq = 末条 seq + 1。
                let text = std::fs::read_to_string(entry.path())
                    .map_err(|e| QueueError(format!("read channel log: {e}")))?;
                let mut last: Option<u64> = None;
                for line in text.lines() {
                    if let Ok(msg) = serde_json::from_str::<QueueMessage>(line) {
                        last = Some(msg.seq);
                    }
                }
                next_seq.insert(channel.to_string(), last.map(|s| s + 1).unwrap_or(0));
            } else if let Some(channel) = name.strip_suffix(".cursor") {
                let text = std::fs::read_to_string(entry.path())
                    .map_err(|e| QueueError(format!("read cursor: {e}")))?;
                let cursor: u64 = text
                    .trim()
                    .parse()
                    .map_err(|e| QueueError(format!("bad cursor: {e}")))?;
                cursors.insert(channel.to_string(), cursor);
            }
        }
        Ok(Self {
            dir,
            next_seq: Mutex::new(next_seq),
            cursors: Mutex::new(cursors),
        })
    }

    fn channel_path(&self, channel: &str) -> PathBuf {
        self.dir.join(format!("{channel}.jsonl"))
    }

    fn cursor_path(&self, channel: &str) -> PathBuf {
        self.dir.join(format!("{channel}.cursor"))
    }
}

impl Seam for FileQueue {}

impl MessageQueue for FileQueue {
    fn publish(&self, channel: &str, payload: Value) -> Result<QueueMessage, QueueError> {
        let mut seqs = self.next_seq.lock().unwrap();
        let seq = seqs.entry(channel.to_string()).or_insert(0);
        let message = QueueMessage {
            channel: channel.to_string(),
            seq: *seq,
            payload,
            ts_ms: now_ms(),
        };
        *seq += 1;
        drop(seqs);

        let line = serde_json::to_string(&message)
            .map_err(|e| QueueError(format!("serialize message: {e}")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.channel_path(channel))
            .map_err(|e| QueueError(format!("open channel: {e}")))?;
        use std::io::Write;
        writeln!(file, "{line}").map_err(|e| QueueError(format!("append message: {e}")))?;
        Ok(message)
    }

    fn consume(&self, channel: &str) -> Result<Option<QueueMessage>, QueueError> {
        let path = self.channel_path(channel);
        if !path.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| QueueError(format!("read channel: {e}")))?;
        // 游标 = 下一个待消费序号(offset 语义)。
        let cursor = self
            .cursors
            .lock()
            .unwrap()
            .get(channel)
            .copied()
            .unwrap_or(0);
        let mut next: Option<QueueMessage> = None;
        for line in text.lines() {
            let message: QueueMessage = serde_json::from_str(line)
                .map_err(|e| QueueError(format!("parse message: {e}")))?;
            if message.seq >= cursor {
                next = Some(message);
                break;
            }
        }
        let Some(message) = next else {
            return Ok(None);
        };
        // 推进游标并持久化(真实偏移提交:下一条待消费序号)。
        let next_offset = message.seq + 1;
        self.cursors
            .lock()
            .unwrap()
            .insert(channel.to_string(), next_offset);
        std::fs::write(self.cursor_path(channel), next_offset.to_string())
            .map_err(|e| QueueError(format!("write cursor: {e}")))?;
        Ok(Some(message))
    }

    fn backlog(&self, channel: &str) -> Result<Vec<QueueMessage>, QueueError> {
        let path = self.channel_path(channel);
        if !path.exists() {
            return Ok(vec![]);
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| QueueError(format!("read channel: {e}")))?;
        let mut messages = Vec::new();
        for line in text.lines() {
            let message: QueueMessage = serde_json::from_str(line)
                .map_err(|e| QueueError(format!("parse message: {e}")))?;
            messages.push(message);
        }
        Ok(messages)
    }

    fn channels(&self) -> Result<Vec<String>, QueueError> {
        let mut names = Vec::new();
        for entry in
            std::fs::read_dir(&self.dir).map_err(|e| QueueError(format!("read queue dir: {e}")))?
        {
            let entry = entry.map_err(|e| QueueError(format!("entry: {e}")))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(channel) = name.strip_suffix(".jsonl") {
                names.push(channel.to_string());
            }
        }
        names.sort();
        Ok(names)
    }
}

/// queue 插件:提供文件后端消息队列。
pub struct QueuePlugin {
    dir: PathBuf,
}

impl QueuePlugin {
    /// 以队列目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for QueuePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-queue"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![QUEUE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let queue = FileQueue::open(self.dir.clone()).map_err(|e| PluginError::Apply {
            plugin: self.name(),
            message: e.0,
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

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(QueuePlugin::new(root.join("queue")))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn publish_consume_in_order() {
        let root = std::env::temp_dir().join(format!("ah-queue-io-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let queue = ctx.service::<dyn MessageQueue>(&QUEUE).expect("queue");

        let m0 = queue.publish("chat", json!({"text": "a"})).expect("p0");
        let m1 = queue.publish("chat", json!({"text": "b"})).expect("p1");
        assert_eq!(m0.seq, 0);
        assert_eq!(m1.seq, 1);

        assert_eq!(
            queue.consume("chat").expect("c0").unwrap().payload["text"],
            "a"
        );
        assert_eq!(
            queue.consume("chat").expect("c1").unwrap().payload["text"],
            "b"
        );
        assert!(queue.consume("chat").expect("c2").is_none(), "drained");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cursor_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-queue-cur-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let queue = ctx.service::<dyn MessageQueue>(&QUEUE).expect("queue");
        queue.publish("chat", json!({"n": 1})).expect("p1");
        queue.publish("chat", json!({"n": 2})).expect("p2");
        assert_eq!(queue.consume("chat").expect("c").unwrap().payload["n"], 1);
        drop(effects);

        // 重开:游标与日志恢复,consume 从第 2 条继续。
        let reopened = FileQueue::open(root.join("queue")).expect("reopen");
        assert_eq!(
            reopened.consume("chat").expect("c2").unwrap().payload["n"],
            2
        );
        assert!(reopened.consume("chat").expect("c3").is_none());
        assert_eq!(
            reopened.backlog("chat").expect("backlog").len(),
            2,
            "log is the truth"
        );
        assert!(
            reopened
                .channels()
                .expect("channels")
                .contains(&"chat".to_string())
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
