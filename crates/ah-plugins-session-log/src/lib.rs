//! # ah-plugins-session-log
//!
//! 真实 append-only 会话事件日志:JSONL 落盘,启动时从文件恢复,
//! 投影(derive_messages)从日志重建模型可见消息(日志即真相)。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::SESSIONS;
use ah_contracts::llm::{ChatMessage, ChatRole, ToolCall};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionError, SessionEvent, SessionEventKind, SessionLog};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 JSONL 会话日志。
pub struct JsonlSessionLog {
    file: Mutex<File>,
    events: Mutex<Vec<SessionEvent>>,
    next_seq: AtomicU64,
    ctx: Context,
}

impl JsonlSessionLog {
    /// 打开(或创建)会话日志文件;文件已存在则恢复全部事件。
    pub fn open(path: impl AsRef<Path>, ctx: Context) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SessionError(format!("create dir failed: {e}")))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SessionError(format!("open session file failed: {e}")))?;

        let mut events = Vec::new();
        if let Ok(reader) = File::open(&path) {
            for line in BufReader::new(reader).lines() {
                let line = line.map_err(|e| SessionError(format!("read line failed: {e}")))?;
                if line.trim().is_empty() {
                    continue;
                }
                let event: SessionEvent = serde_json::from_str(&line)
                    .map_err(|e| SessionError(format!("invalid session line: {e}")))?;
                events.push(event);
            }
        }

        let next_seq = events.last().map(|e| e.seq + 1).unwrap_or(0);
        Ok(Self {
            file: Mutex::new(file),
            events: Mutex::new(events),
            next_seq: AtomicU64::new(next_seq),
            ctx,
        })
    }

    /// 测试辅助:临时会话文件路径。
    #[cfg(test)]
    pub fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ah-session-{tag}-{}", std::process::id()))
    }
}

impl Seam for JsonlSessionLog {}

impl SessionLog for JsonlSessionLog {
    fn append(&self, kind: SessionEventKind, payload: Value) -> Result<SessionEvent, SessionError> {
        let event = SessionEvent {
            seq: self.next_seq.fetch_add(1, Ordering::SeqCst),
            timestamp_ms: now_ms(),
            kind,
            payload,
        };
        let line = serde_json::to_string(&event)
            .map_err(|e| SessionError(format!("serialize event failed: {e}")))?;

        // 真实落盘(append 一行 JSONL)。
        {
            let mut file = self.file.lock().unwrap();
            writeln!(file, "{line}").map_err(|e| SessionError(format!("append failed: {e}")))?;
            file.flush()
                .map_err(|e| SessionError(format!("flush failed: {e}")))?;
        }

        self.events.lock().unwrap().push(event.clone());
        // 广播:持久化事实实时可见。
        self.ctx.emit(event.clone());
        Ok(event)
    }

    fn events(&self) -> Vec<SessionEvent> {
        self.events.lock().unwrap().clone()
    }

    fn since(&self, seq: u64) -> Vec<SessionEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.seq > seq)
            .cloned()
            .collect()
    }

    fn derive_messages(&self) -> Vec<ChatMessage> {
        self.events()
            .into_iter()
            .filter_map(|event| match event.kind {
                SessionEventKind::User => event
                    .payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|c| ChatMessage::new(ChatRole::User, c)),
                SessionEventKind::Assistant => {
                    if let Some(calls) = event.payload.get("tool_calls").and_then(Value::as_array) {
                        let tool_calls: Vec<ToolCall> = calls
                            .iter()
                            .filter_map(|call| {
                                Some(ToolCall {
                                    id: call.get("id")?.as_str()?.to_string(),
                                    name: call.get("name")?.as_str()?.to_string(),
                                    arguments: call.get("arguments")?.clone(),
                                })
                            })
                            .collect();
                        if tool_calls.is_empty() {
                            None
                        } else {
                            Some(ChatMessage::assistant_with_tool_calls(tool_calls))
                        }
                    } else {
                        event
                            .payload
                            .get("content")
                            .and_then(Value::as_str)
                            .map(|c| ChatMessage::new(ChatRole::Assistant, c))
                    }
                }
                SessionEventKind::ToolResult => {
                    let id = event.payload.get("tool_call_id").and_then(Value::as_str)?;
                    let output = event
                        .payload
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some(ChatMessage::tool(id, output))
                }
                _ => None,
            })
            .collect()
    }
}

/// 会话日志插件:提供 sessions seam。
pub struct SessionLogPlugin {
    path: PathBuf,
}

impl SessionLogPlugin {
    /// 以日志文件路径创建插件。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Plugin for SessionLogPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-session-log"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SESSIONS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let log =
            JsonlSessionLog::open(&self.path, ctx.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        Ok(vec![
            ctx.register(SESSIONS, Arc::new(log) as Arc<dyn SessionLog>),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_log(tag: &str) -> (JsonlSessionLog, PathBuf) {
        let path = JsonlSessionLog::temp_path(tag);
        let _ = std::fs::remove_file(&path);
        let log = JsonlSessionLog::open(&path, Context::new()).expect("open");
        (log, path)
    }

    #[test]
    fn append_events_and_reload_from_disk() {
        let (log, path) = make_log("roundtrip");
        log.append(SessionEventKind::User, json!({ "content": "hi" }))
            .expect("append");
        log.append(SessionEventKind::Assistant, json!({ "content": "hello" }))
            .expect("append");
        drop(log);

        // 从同一文件重新打开:真实 JSONL 落盘往返。
        let reloaded = JsonlSessionLog::open(&path, Context::new()).expect("reopen");
        let events = reloaded.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[0].kind, SessionEventKind::User);
        assert_eq!(events[0].payload["content"], "hi");
        assert_eq!(events[1].seq, 1);
        assert_eq!(events[1].kind, SessionEventKind::Assistant);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn since_returns_incremental_events() {
        let (log, path) = make_log("since");
        log.append(SessionEventKind::User, json!({ "content": "a" }))
            .expect("1");
        log.append(SessionEventKind::User, json!({ "content": "b" }))
            .expect("2");
        let incremental = log.since(0);
        assert_eq!(incremental.len(), 1);
        assert_eq!(incremental[0].payload["content"], "b");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn derive_messages_projects_log_to_model_view() {
        let (log, path) = make_log("derive");
        log.append(SessionEventKind::User, json!({ "content": "hi" }))
            .expect("user");
        log.append(
            SessionEventKind::Assistant,
            json!({
                "tool_calls": [{
                    "id": "call-1",
                    "name": "list_dir",
                    "arguments": { "path": "." }
                }]
            }),
        )
        .expect("assistant tool calls");
        log.append(
            SessionEventKind::ToolResult,
            json!({ "tool_call_id": "call-1", "output": "{\"entries\":[]}" }),
        )
        .expect("tool result");
        log.append(SessionEventKind::Assistant, json!({ "content": "done" }))
            .expect("final");

        let messages = log.derive_messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, ChatRole::User);
        assert_eq!(messages[0].content, "hi");
        assert_eq!(messages[1].role, ChatRole::Assistant);
        let calls = messages[1].tool_calls.as_ref().expect("tool calls");
        assert_eq!(calls[0].name, "list_dir");
        assert_eq!(messages[2].role, ChatRole::Tool);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(messages[3].content, "done");

        let _ = std::fs::remove_file(&path);
    }
}
