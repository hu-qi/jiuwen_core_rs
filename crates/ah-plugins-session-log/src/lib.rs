//! # ah-plugins-session-log
//!
//! 真实 append-only 会话事件日志:JSONL 落盘,启动时从文件恢复,
//! 投影(derive_messages)从日志重建模型可见消息(日志即真相)。
//! 提供 sessions(默认会话)与 session-manager(多会话 create/open/fork/list)。

const SESSION_LOG_VERSION: u64 = 1;

fn decode_session_line(line: &str) -> Result<SessionEvent, SessionError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| SessionError(format!("invalid session line: {e}")))?;
    let event_value = if let Some(version) = value.get("version") {
        if version.as_u64() != Some(SESSION_LOG_VERSION) {
            return Err(SessionError(format!(
                "unsupported session log version: {}",
                version
            )));
        }
        value
            .get("event")
            .cloned()
            .ok_or_else(|| SessionError("versioned session line missing event".into()))?
    } else {
        value
    };
    let event: SessionEvent = serde_json::from_value(event_value)
        .map_err(|e| SessionError(format!("invalid session event: {e}")))?;
    validate_session_event(&event)?;
    Ok(event)
}

fn validate_session_event(event: &SessionEvent) -> Result<(), SessionError> {
    match event.kind {
        SessionEventKind::Assistant => {
            if let Some(calls) = event.payload.get("tool_calls") {
                let calls = calls
                    .as_array()
                    .ok_or_else(|| SessionError("assistant tool_calls must be an array".into()))?;
                let mut ids = Vec::with_capacity(calls.len());
                for call in calls {
                    let parsed: ToolCall = serde_json::from_value(call.clone())
                        .map_err(|e| SessionError(format!("invalid assistant tool call: {e}")))?;
                    if parsed.id.trim().is_empty() || parsed.name.trim().is_empty() {
                        return Err(SessionError(
                            "assistant tool call id and name must not be empty".into(),
                        ));
                    }
                    if ids.iter().any(|id: &String| id == &parsed.id) {
                        return Err(SessionError(format!(
                            "duplicate assistant tool call id: {}",
                            parsed.id
                        )));
                    }
                    ids.push(parsed.id);
                }
            } else if event
                .payload
                .get("content")
                .and_then(Value::as_str)
                .is_none()
            {
                return Err(SessionError(
                    "assistant event requires content or tool_calls".into(),
                ));
            }
        }
        SessionEventKind::ToolResult => {
            let id = event
                .payload
                .get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| SessionError("tool result requires tool_call_id".into()))?;
            if let Some(images) = event.payload.get("images") {
                serde_json::from_value::<Vec<ChatImage>>(images.clone()).map_err(|e| {
                    SessionError(format!("tool result {id} has invalid images: {e}"))
                })?;
            }
            if event
                .payload
                .get("output")
                .and_then(Value::as_str)
                .is_none()
            {
                return Err(SessionError(format!(
                    "tool result {id} requires string output"
                )));
            }
            if let Some(status) = event.payload.get("status") {
                match status.as_str() {
                    Some("unknown") | Some("completed") | Some("error") => {}
                    _ => return Err(SessionError("invalid tool result status".into())),
                }
            }
        }
        _ => {}
    }
    Ok(())
}

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::{SESSION_MANAGER, SESSIONS};
use ah_contracts::llm::{ChatImage, ChatMessage, ChatRole, ToolCall};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{
    SessionError, SessionEvent, SessionEventKind, SessionLog, SessionManager,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

fn extract_image_attachment(output: &str) -> Vec<ChatImage> {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    let Some(mime_type) = value.get("mime_type").and_then(Value::as_str) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(Value::as_str) else {
        return Vec::new();
    };
    if !mime_type.starts_with("image/") || data.is_empty() {
        return Vec::new();
    }
    vec![ChatImage::new(mime_type, data)]
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn validate_events(events: &[SessionEvent]) -> Result<(), SessionError> {
    for (expected_seq, event) in events.iter().enumerate() {
        if event.seq != expected_seq as u64 {
            return Err(SessionError(format!(
                "session event sequence is not contiguous: expected {}, got {}",
                expected_seq, event.seq
            )));
        }
    }
    Ok(())
}

/// Read complete JSONL records and discard only a torn final record.
fn load_events(file: &mut File) -> Result<Vec<SessionEvent>, SessionError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| SessionError(format!("seek session file failed: {e}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| SessionError(format!("read session file failed: {e}")))?;

    let mut events = Vec::new();
    let mut cursor = 0;
    while let Some(relative_end) = bytes[cursor..].iter().position(|byte| *byte == b'\n') {
        let end = cursor + relative_end;
        let line = std::str::from_utf8(&bytes[cursor..end])
            .map_err(|e| SessionError(format!("invalid session line encoding: {e}")))?;
        if !line.trim().is_empty() {
            events.push(decode_session_line(line)?);
        }
        cursor = end + 1;
    }

    // A crash can leave a final record without its terminating newline. If it
    // is valid, normalize it; if it is invalid, discard only that final tail.
    if cursor < bytes.len() {
        let tail = &bytes[cursor..];
        let tail_text = std::str::from_utf8(tail).ok();
        if tail_text.is_some_and(|text| text.trim().is_empty()) {
            file.set_len(cursor as u64)
                .map_err(|e| SessionError(format!("truncate session tail failed: {e}")))?;
            file.sync_data()
                .map_err(|e| SessionError(format!("sync repaired session failed: {e}")))?;
        } else if let Some(text) = tail_text {
            match decode_session_line(text) {
                Ok(event) => {
                    events.push(event);
                    file.seek(SeekFrom::End(0))
                        .map_err(|e| SessionError(format!("seek session tail failed: {e}")))?;
                    file.write_all(b"\n")
                        .map_err(|e| SessionError(format!("terminate session line failed: {e}")))?;
                    file.flush()
                        .map_err(|e| SessionError(format!("flush repaired session failed: {e}")))?;
                    file.sync_data()
                        .map_err(|e| SessionError(format!("sync repaired session failed: {e}")))?;
                }
                Err(error) if error.0.contains("unsupported session log version") => {
                    return Err(error);
                }
                Err(error)
                    if text.trim_start().starts_with('{')
                        && (text.contains("\"version\"") || text.contains("\"event\"")) =>
                {
                    // A JSON-looking record with the session envelope but no
                    // complete object is a torn crash tail, not a valid event.
                    file.set_len(cursor as u64).map_err(|e| {
                        SessionError(format!("truncate incomplete session line failed: {e}"))
                    })?;
                    file.sync_data()
                        .map_err(|e| SessionError(format!("sync truncated session failed: {e}")))?;
                }
                Err(error) => return Err(error),
            }
        } else {
            file.set_len(cursor as u64).map_err(|e| {
                SessionError(format!("truncate incomplete session encoding failed: {e}"))
            })?;
            file.sync_data().map_err(|e| {
                SessionError(format!("sync truncated session encoding failed: {e}"))
            })?;
        }
    }
    validate_events(&events)?;
    file.seek(SeekFrom::End(0))
        .map_err(|e| SessionError(format!("seek session append position failed: {e}")))?;
    Ok(events)
}

struct LockedFile {
    file: File,
}

impl LockedFile {
    fn open(path: &Path) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SessionError(format!("create dir failed: {e}")))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(|e| SessionError(format!("open session file failed: {e}")))?;
        #[cfg(unix)]
        {
            // SAFETY: flock only borrows this valid open file descriptor and
            // the guard releases it in Drop, including during panic unwinding.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(SessionError(format!(
                    "lock session file failed: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }
        Ok(Self { file })
    }
}

impl Drop for LockedFile {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY: this is the descriptor locked by LockedFile::open.
            unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

fn atomic_copy(src: &Path, dst: &Path) -> Result<(), SessionError> {
    let parent = dst
        .parent()
        .ok_or_else(|| SessionError("destination has no parent directory".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| SessionError(format!("create destination dir failed: {e}")))?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}-{}",
        dst.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("session"),
        std::process::id(),
        now_ms()
    ));
    let result = (|| {
        let mut input =
            File::open(src).map_err(|e| SessionError(format!("open copy source failed: {e}")))?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|e| SessionError(format!("create atomic copy failed: {e}")))?;
        std::io::copy(&mut input, &mut output)
            .map_err(|e| SessionError(format!("copy session snapshot failed: {e}")))?;
        output
            .flush()
            .map_err(|e| SessionError(format!("flush session snapshot failed: {e}")))?;
        output
            .sync_all()
            .map_err(|e| SessionError(format!("sync session snapshot failed: {e}")))?;
        drop(output);
        std::fs::rename(&tmp, dst)
            .map_err(|e| SessionError(format!("replace session snapshot failed: {e}")))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 真实 JSONL 会话日志。
pub struct JsonlSessionLog {
    id: String,
    path: PathBuf,
    ctx: Context,
}

impl JsonlSessionLog {
    /// 打开(或创建)会话日志文件;文件已存在则恢复全部事件。
    pub fn open(path: impl AsRef<Path>, ctx: Context) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        let mut locked = LockedFile::open(&path)?;
        let _ = load_events(&mut locked.file)?;
        drop(locked);
        let id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("default")
            .to_string();
        Ok(Self { id, path, ctx })
    }

    /// 测试辅助:临时会话文件路径。
    #[cfg(test)]
    pub fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ah-session-{tag}-{}", std::process::id()))
    }
}

impl Seam for JsonlSessionLog {}

impl SessionLog for JsonlSessionLog {
    fn id(&self) -> &str {
        &self.id
    }

    fn append(&self, kind: SessionEventKind, payload: Value) -> Result<SessionEvent, SessionError> {
        let mut locked = LockedFile::open(&self.path)?;
        let events = load_events(&mut locked.file)?;
        let seq = events.last().map(|event| event.seq + 1).unwrap_or(0);
        let event = SessionEvent {
            seq,
            timestamp_ms: now_ms(),
            kind,
            payload,
        };
        let line = serde_json::to_string(&json!({
            "version": SESSION_LOG_VERSION,
            "event": event,
        }))
        .map_err(|e| SessionError(format!("serialize event failed: {e}")))?;

        locked
            .file
            .write_all(line.as_bytes())
            .and_then(|_| locked.file.write_all(b"\n"))
            .map_err(|e| SessionError(format!("append failed: {e}")))?;
        locked
            .file
            .flush()
            .map_err(|e| SessionError(format!("flush failed: {e}")))?;
        locked
            .file
            .sync_data()
            .map_err(|e| SessionError(format!("sync failed: {e}")))?;
        drop(locked);

        // 广播:持久化事实实时可见。
        self.ctx.emit(event.clone());
        Ok(event)
    }

    fn try_events(&self) -> Result<Vec<SessionEvent>, SessionError> {
        let mut locked = LockedFile::open(&self.path)?;
        load_events(&mut locked.file)
    }
    fn claim_tool_call(
        &self,
        call_id: &str,
        owner: &str,
        lease_ms: u64,
    ) -> Result<bool, SessionError> {
        if call_id.trim().is_empty() || owner.trim().is_empty() || lease_ms == 0 {
            return Err(SessionError("invalid tool call claim".into()));
        }
        let mut locked = LockedFile::open(&self.path)?;
        let events = load_events(&mut locked.file)?;
        let now = now_ms();
        for event in &events {
            if event.kind == SessionEventKind::ToolResult
                && event.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
            {
                return Ok(false);
            }
            if event.kind == SessionEventKind::System
                && event.payload.get("event").and_then(Value::as_str) == Some("tool_call_claim")
                && event.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
                && event
                    .payload
                    .get("lease_until_ms")
                    .and_then(Value::as_u64)
                    .is_some_and(|lease_until| lease_until > now)
            {
                return Ok(false);
            }
        }
        let event = SessionEvent {
            seq: events.last().map(|event| event.seq + 1).unwrap_or(0),
            timestamp_ms: now,
            kind: SessionEventKind::System,
            payload: json!({
                "event": "tool_call_claim",
                "tool_call_id": call_id,
                "owner": owner,
                "lease_until_ms": now.saturating_add(lease_ms),
            }),
        };
        let line = serde_json::to_string(&json!({
            "version": SESSION_LOG_VERSION,
            "event": event,
        }))
        .map_err(|e| SessionError(format!("serialize tool claim failed: {e}")))?;
        locked
            .file
            .write_all(line.as_bytes())
            .and_then(|_| locked.file.write_all(b"\n"))
            .map_err(|e| SessionError(format!("append tool claim failed: {e}")))?;
        locked
            .file
            .flush()
            .map_err(|e| SessionError(format!("flush tool claim failed: {e}")))?;
        locked
            .file
            .sync_data()
            .map_err(|e| SessionError(format!("sync tool claim failed: {e}")))?;
        drop(locked);
        self.ctx.emit(event);
        Ok(true)
    }

    fn events(&self) -> Vec<SessionEvent> {
        self.try_events().unwrap_or_default()
    }

    fn since(&self, seq: u64) -> Vec<SessionEvent> {
        self.events()
            .into_iter()
            .filter(|event| event.seq > seq)
            .collect()
    }

    fn derive_messages(&self) -> Vec<ChatMessage> {
        self.events()
            .into_iter()
            .filter_map(|event| match event.kind {
                SessionEventKind::User => {
                    let content = event
                        .payload
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let mut message = ChatMessage::new(ChatRole::User, content);
                    message.images = event
                        .payload
                        .get("images")
                        .and_then(|images| serde_json::from_value(images.clone()).ok())
                        .unwrap_or_default();
                    if message.content.is_empty() && message.images.is_empty() {
                        None
                    } else {
                        Some(message)
                    }
                },
                SessionEventKind::Assistant => {
                    let reasoning_content = event
                        .payload
                        .get("reasoning_content")
                        .and_then(Value::as_str)
                        .map(str::to_string);
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
                            let mut message = ChatMessage::assistant_with_tool_calls(tool_calls);
                            message.reasoning_content = reasoning_content;
                            Some(message)
                        }
                    } else {
                        event
                            .payload
                            .get("content")
                            .and_then(Value::as_str)
                            .map(|c| {
                                let mut message = ChatMessage::new(ChatRole::Assistant, c);
                                message.reasoning_content = reasoning_content;
                                message
                            })
                    }
                }
                SessionEventKind::ToolResult => {
                    let id = event.payload.get("tool_call_id").and_then(Value::as_str)?;
                    let raw_output = event
                        .payload
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let output = if event.payload.get("status").and_then(Value::as_str)
                        == Some("unknown")
                    {
                        format!(
                            "[tool result status=unknown; manual reconciliation required]\n{raw_output}"
                        )
                    } else {
                        raw_output.to_string()
                    };
                    let mut message = ChatMessage::tool(id, output);
                    message.images = event
                        .payload
                        .get("images")
                        .and_then(|images| serde_json::from_value(images.clone()).ok())
                        .unwrap_or_else(|| extract_image_attachment(raw_output));
                    Some(message)
                }
                SessionEventKind::System => event
                    .payload
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|c| ChatMessage::new(ChatRole::System, c)),
                _ => None,
            })
            .collect()
    }
}

/// 会话管理器:按 id 管理独立 JSONL 会话文件(dir/{id}.jsonl)。
pub struct SessionManagerImpl {
    dir: PathBuf,
    ctx: Context,
}

impl SessionManagerImpl {
    /// 以会话目录创建管理器。
    pub fn new(dir: impl Into<PathBuf>, ctx: Context) -> Result<Self, SessionError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| SessionError(format!("create session dir failed: {e}")))?;
        Ok(Self { dir, ctx })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.jsonl"))
    }

    fn checkpoint_path(&self, name: &str) -> PathBuf {
        self.dir.join("checkpoints").join(format!("{name}.jsonl"))
    }

    fn validate_checkpoint_name(name: &str) -> Result<(), SessionError> {
        if name.trim().is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
        {
            return Err(SessionError("invalid checkpoint name".into()));
        }
        Ok(())
    }
}

impl Seam for SessionManagerImpl {}

impl SessionManager for SessionManagerImpl {
    fn create(&self, id: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError> {
        let log = JsonlSessionLog::open(self.path_for(id), self.ctx.clone())?;
        Ok(std::sync::Arc::new(log) as std::sync::Arc<dyn SessionLog>)
    }

    fn open(&self, id: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError> {
        let path = self.path_for(id);
        if !path.exists() {
            return Err(SessionError(format!("session not found: {id}")));
        }
        let log = JsonlSessionLog::open(path, self.ctx.clone())?;
        Ok(std::sync::Arc::new(log) as std::sync::Arc<dyn SessionLog>)
    }

    fn fork(&self, from: &str, to: &str) -> Result<std::sync::Arc<dyn SessionLog>, SessionError> {
        let src = self.path_for(from);
        let dst = self.path_for(to);
        if !src.exists() {
            return Err(SessionError(format!("session not found: {from}")));
        }
        let mut source = LockedFile::open(&src)?;
        let _ = load_events(&mut source.file)?;
        atomic_copy(&src, &dst)?;
        drop(source);
        let log = JsonlSessionLog::open(dst, self.ctx.clone())?;
        Ok(Arc::new(log) as Arc<dyn SessionLog>)
    }

    fn checkpoint(&self, id: &str, name: &str) -> Result<(), SessionError> {
        Self::validate_checkpoint_name(name)?;
        let src = self.path_for(id);
        if !src.exists() {
            return Err(SessionError(format!("session not found: {id}")));
        }
        let dst = self.checkpoint_path(name);
        let mut source = LockedFile::open(&src)?;
        let _ = load_events(&mut source.file)?;
        atomic_copy(&src, &dst)?;
        Ok(())
    }

    fn restore(
        &self,
        id: &str,
        name: &str,
    ) -> Result<std::sync::Arc<dyn SessionLog>, SessionError> {
        Self::validate_checkpoint_name(name)?;
        let src = self.checkpoint_path(name);
        if !src.exists() {
            return Err(SessionError(format!("checkpoint not found: {name}")));
        }
        // Validate and repair a torn checkpoint before replacing the session.
        let mut checkpoint = LockedFile::open(&src)?;
        let _ = load_events(&mut checkpoint.file)?;
        drop(checkpoint);

        let dst = self.path_for(id);
        let destination = LockedFile::open(&dst)?;
        atomic_copy(&src, &dst)?;
        drop(destination);
        let log = JsonlSessionLog::open(dst, self.ctx.clone())?;
        Ok(Arc::new(log) as Arc<dyn SessionLog>)
    }

    fn list(&self) -> Vec<String> {
        let mut ids: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        name.strip_suffix(".jsonl").map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }
}

/// 会话日志插件:提供 sessions(默认会话)与 session-manager(多会话管理)。
pub struct SessionLogPlugin {
    path: PathBuf,
    dir: PathBuf,
}

impl SessionLogPlugin {
    /// 以默认会话文件与会话目录创建插件。
    pub fn new(path: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            dir: dir.into(),
        }
    }
}

impl Plugin for SessionLogPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-session-log"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SESSIONS, SESSION_MANAGER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let log =
            JsonlSessionLog::open(&self.path, ctx.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let manager =
            SessionManagerImpl::new(&self.dir, ctx.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        Ok(vec![
            ctx.register(
                SESSIONS,
                std::sync::Arc::new(log) as std::sync::Arc<dyn SessionLog>,
            ),
            ctx.register(
                SESSION_MANAGER,
                std::sync::Arc::new(manager) as std::sync::Arc<dyn SessionManager>,
            ),
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
                }],
                "reasoning_content": "inspect the workspace first"
            }),
        )
        .expect("assistant tool calls");
        log.append(
            SessionEventKind::ToolResult,
            json!({ "tool_call_id": "call-1", "output": "{\"entries\":[]}" }),
        )
        .expect("tool result");
        log.append(
            SessionEventKind::Assistant,
            json!({
                "content": "done",
                "reasoning_content": "the directory is empty"
            }),
        )
        .expect("final");

        let messages = log.derive_messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, ChatRole::User);
        assert_eq!(messages[0].content, "hi");
        assert_eq!(messages[1].role, ChatRole::Assistant);
        let calls = messages[1].tool_calls.as_ref().expect("tool calls");
        assert_eq!(
            messages[1].reasoning_content.as_deref(),
            Some("inspect the workspace first")
        );
        assert_eq!(calls[0].name, "list_dir");
        assert_eq!(messages[2].role, ChatRole::Tool);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(messages[3].content, "done");
        assert_eq!(
            messages[3].reasoning_content.as_deref(),
            Some("the directory is empty")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_tool_result_projection_requires_reconciliation() {
        let (log, path) = make_log("unknown-result");
        log.append(
            SessionEventKind::ToolResult,
            json!({
                "tool_call_id": "call-unknown",
                "output": "execution outcome unavailable",
                "status": "unknown"
            }),
        )
        .expect("unknown tool result");

        let messages = log.derive_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::Tool);
        assert!(
            messages[0]
                .content
                .contains("manual reconciliation required")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn manager_create_open_fork_list() {
        let dir = std::env::temp_dir().join(format!("ah-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let manager = SessionManagerImpl::new(&dir, Context::new()).expect("manager");

        // create 并写入
        let a = manager.create("a").expect("create a");
        a.append(SessionEventKind::User, json!({ "content": "one" }))
            .expect("append");

        // list / open 恢复
        assert_eq!(manager.list(), vec!["a".to_string()]);
        let opened = manager.open("a").expect("open a");
        assert_eq!(opened.events().len(), 1);

        // fork 复制历史;原会话继续写不影响 fork 出的
        let b = manager.fork("a", "b").expect("fork b");
        assert_eq!(b.events().len(), 1);
        a.append(SessionEventKind::User, json!({ "content": "two" }))
            .expect("append a2");
        assert_eq!(a.events().len(), 2);
        assert_eq!(b.events().len(), 1, "fork 出的会话不受原会话影响");
        assert_eq!(manager.list(), vec!["a".to_string(), "b".to_string()]);

        // open 不存在报错
        assert!(manager.open("missing").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_and_restore_rolls_back_session() {
        let dir = std::env::temp_dir().join(format!("ah-session-cp-{}", std::process::id()));
        let manager = SessionManagerImpl::new(&dir, Context::new()).expect("manager");

        let log = manager.create("s1").expect("create");
        log.append(SessionEventKind::User, json!({ "content": "one" }))
            .expect("1");
        log.append(SessionEventKind::Assistant, json!({ "content": "two" }))
            .expect("2");
        manager.checkpoint("s1", "cp1").expect("checkpoint");

        // 检查点之后继续写入。
        log.append(SessionEventKind::User, json!({ "content": "three" }))
            .expect("3");
        assert_eq!(manager.open("s1").expect("open").events().len(), 3);

        // 从检查点恢复:回到 2 条事件,seq 从 2 继续。
        let restored = manager.restore("s1", "cp1").expect("restore");
        let events = restored.events();
        assert_eq!(events.len(), 2, "rolled back to checkpoint");
        assert_eq!(events[0].payload["content"], "one");
        restored
            .append(
                SessionEventKind::Assistant,
                json!({ "content": "after restore" }),
            )
            .expect("append after restore");
        let events = restored.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].seq, 2, "seq continues from checkpoint");

        // 不存在的检查点报错。
        assert!(manager.restore("s1", "missing").is_err());
        assert!(manager.checkpoint("missing", "cp2").is_err());
        assert!(manager.checkpoint("s1", "../escape").is_err());
        assert!(manager.restore("s1", "../escape").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn recovers_torn_final_line_and_continues_sequence() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-torn-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let first = json!({
            "version": SESSION_LOG_VERSION,
            "event": {
                "seq": 0,
                "timestamp_ms": 1,
                "kind": "user",
                "payload": {"content": "kept"}
            }
        });
        std::fs::write(
            &path,
            format!(
                "{}\n{{\"version\":1,\"event\":{{\"seq\":1,\"kind\":\"assistant\"",
                first
            ),
        )
        .expect("write torn log");

        let log = JsonlSessionLog::open(&path, Context::new()).expect("repair torn log");
        assert_eq!(log.events().len(), 1);
        let appended = log
            .append(SessionEventKind::Assistant, json!({"content": "next"}))
            .expect("append after repair");
        assert_eq!(appended.seq, 1);
        assert_eq!(
            JsonlSessionLog::open(&path, Context::new())
                .unwrap()
                .events()
                .len(),
            2
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_corrupt_complete_line() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-corrupt-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"{not-json}\n").expect("write corrupt log");
        let error = match JsonlSessionLog::open(&path, Context::new()) {
            Ok(_) => panic!("corruption accepted"),
            Err(error) => error,
        };
        assert!(error.0.contains("invalid session line"));
        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn cross_process_writer_child() {
        let Some(path) = std::env::var_os("AH_SESSION_CHILD_PATH") else {
            return;
        };
        let worker: usize = std::env::var("AH_SESSION_CHILD_WORKER")
            .expect("worker")
            .parse()
            .expect("worker number");
        let count: usize = std::env::var("AH_SESSION_CHILD_COUNT")
            .expect("count")
            .parse()
            .expect("count number");
        let log = JsonlSessionLog::open(path, Context::new()).expect("open child log");
        for index in 0..count {
            log.append(
                SessionEventKind::User,
                json!({"content": format!("{worker}-{index}")}),
            )
            .expect("child append");
        }
    }

    #[cfg(unix)]
    #[test]
    fn cross_process_appenders_keep_unique_contiguous_sequences() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-process-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let exe = std::env::current_exe().expect("test executable");
        let workers = 4;
        let per_worker = 16;
        let mut children = Vec::new();
        for worker in 0..workers {
            children.push(
                std::process::Command::new(&exe)
                    .args([
                        "--exact",
                        "tests::cross_process_writer_child",
                        "--nocapture",
                    ])
                    .env("AH_SESSION_CHILD_PATH", &path)
                    .env("AH_SESSION_CHILD_WORKER", worker.to_string())
                    .env("AH_SESSION_CHILD_COUNT", per_worker.to_string())
                    .spawn()
                    .expect("spawn child writer"),
            );
        }
        for mut child in children {
            assert!(child.wait().expect("wait child writer").success());
        }

        let log = JsonlSessionLog::open(&path, Context::new()).expect("reopen process log");
        let events = log.events();
        assert_eq!(events.len(), workers * per_worker);
        for (expected, event) in events.iter().enumerate() {
            assert_eq!(event.seq, expected as u64);
        }
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn rejects_semantically_incomplete_tool_call() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-incomplete-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let raw = json!({
            "version": SESSION_LOG_VERSION,
            "event": {
                "seq": 0,
                "timestamp_ms": 1,
                "kind": "assistant",
                "payload": {"tool_calls": [{"id": "call-1", "name": "write_file"}]}
            }
        });
        std::fs::write(&path, format!("{raw}\n")).expect("write incomplete call");
        let error = match JsonlSessionLog::open(&path, Context::new()) {
            Ok(_) => panic!("incomplete call accepted"),
            Err(error) => error,
        };
        assert!(error.0.contains("invalid assistant tool call"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn try_events_reports_corruption_in_existing_handle() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-read-error-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let log = JsonlSessionLog::open(&path, Context::new()).expect("open session");
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open corrupted tail");
        writeln!(file, "{{not-json}}").expect("write corruption");
        let error = match log.try_events() {
            Ok(_) => panic!("corruption hidden by try_events"),
            Err(error) => error,
        };
        assert!(error.0.contains("invalid session line"));
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn tool_call_claim_is_single_owner_and_completion_closes_claim() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-claim-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let first = JsonlSessionLog::open(&path, Context::new()).expect("first handle");
        let second = JsonlSessionLog::open(&path, Context::new()).expect("second handle");
        first
            .append(
                SessionEventKind::Assistant,
                json!({"tool_calls": [{"id": "claim-1", "name": "list_dir", "arguments": {}}]}),
            )
            .expect("pending call");
        assert!(
            first
                .claim_tool_call("claim-1", "owner-a", 60_000)
                .expect("first claim")
        );
        assert!(
            !second
                .claim_tool_call("claim-1", "owner-b", 60_000)
                .expect("second claim")
        );
        first
            .append(
                SessionEventKind::ToolResult,
                json!({"tool_call_id": "claim-1", "output": "ok"}),
            )
            .expect("complete call");
        assert!(
            !second
                .claim_tool_call("claim-1", "owner-b", 60_000)
                .expect("claim completed call")
        );
        let _ = std::fs::remove_file(path);
    }
    #[cfg(unix)]
    #[test]
    fn cross_process_claim_child() {
        let Some(path) = std::env::var_os("AH_SESSION_CLAIM_PATH") else {
            return;
        };
        let owner = std::env::var("AH_SESSION_CLAIM_OWNER").expect("claim owner");
        let log = JsonlSessionLog::open(path, Context::new()).expect("open claim session");
        if log
            .claim_tool_call("process-claim", &owner, 60_000)
            .expect("claim tool call")
        {
            log.append(
                SessionEventKind::ToolResult,
                json!({"tool_call_id": "process-claim", "output": "claimed"}),
            )
            .expect("append claim result");
        }
    }

    #[cfg(unix)]
    #[test]
    fn cross_process_claim_has_one_owner() {
        let path = std::env::temp_dir().join(format!(
            "ah-session-claim-process-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_file(&path);
        let log = JsonlSessionLog::open(&path, Context::new()).expect("open claim session");
        log.append(
            SessionEventKind::Assistant,
            json!({"tool_calls": [{"id": "process-claim", "name": "list_dir", "arguments": {}}]}),
        )
        .expect("append pending call");
        drop(log);

        let exe = std::env::current_exe().expect("test executable");
        let mut children = Vec::new();
        for worker in 0..4 {
            children.push(
                std::process::Command::new(&exe)
                    .args(["--exact", "tests::cross_process_claim_child", "--nocapture"])
                    .env("AH_SESSION_CLAIM_PATH", &path)
                    .env("AH_SESSION_CLAIM_OWNER", format!("owner-{worker}"))
                    .spawn()
                    .expect("spawn claim worker"),
            );
        }
        for mut child in children {
            assert!(child.wait().expect("wait claim worker").success());
        }
        let log = JsonlSessionLog::open(&path, Context::new()).expect("reopen claim session");
        let events = log.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| {
                    event.kind == SessionEventKind::System
                        && event.payload["event"] == "tool_call_claim"
                })
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == SessionEventKind::ToolResult)
                .count(),
            1
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn derive_messages_restores_tool_image_attachment() {
        let (log, path) = make_log("tool-image");
        log.append(
            SessionEventKind::ToolResult,
            json!({
                "tool_call_id": "screenshot-1",
                "output": "{\"mime_type\":\"image/png\"}",
                "images": [{"mime_type":"image/png","data":"AAAA"}]
            }),
        )
        .expect("tool image");
        let messages = log.derive_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].images[0].data, "AAAA");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn derive_messages_restores_user_observation_image() {
        let (log, path) = make_log("user-image");
        log.append(
            SessionEventKind::User,
            json!({"content":"[current Android screen]","images":[{"mime_type":"image/png","data":"AAAA"}]}),
        )
        .expect("observation");
        let messages = log.derive_messages();
        assert_eq!(messages[0].images[0].mime_type, "image/png");
        let _ = std::fs::remove_file(path);
    }
}
