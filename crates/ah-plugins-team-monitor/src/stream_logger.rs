//! 团队流式输出的聚合诊断日志(对齐 Python agent_teams/monitor/stream_logger.py
//! `TeamStreamLogger`)。
//!
//! 语义:token 流 chunk(``llm_output`` / ``llm_reasoning``)按 ``(member, role)``
//! 来源**独立缓冲**成一段连续 run,来源切换类别、出现离散 chunk 或流结束时
//! 落盘为一条多行记录;其余 chunk 类型立即写入。`answer` 与同一来源已出现过的
//! `llm_output` 去重;未打团队标签(infra 层透传)的 chunk 跳过。
//!
//! `feed`/`flush` 自身吞掉异常(诊断日志不能打断被观察的流),构造时路径不可用
//! 则显式失败。

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ah_contracts::stream::TeamOutputSchema;
use serde_json::Value;

// Chunk type 常量(对齐 stream_logger.py `_CHUNK_*`)。
pub const CHUNK_LLM_OUTPUT: &str = "llm_output";
pub const CHUNK_LLM_REASONING: &str = "llm_reasoning";
pub const CHUNK_ANSWER: &str = "answer";
pub const CHUNK_INTERACTION: &str = "__interaction__";
pub const CHUNK_MESSAGE: &str = "message";
pub const CHUNK_TOOL_CALL: &str = "tool_call";
pub const CHUNK_TOOL_RESULT: &str = "tool_result";
pub const CHUNK_TOOL_UPDATE: &str = "tool_update";
pub const CHUNK_TODO_UPDATED: &str = "todo.updated";
pub const CHUNK_CONTROLLER_OUTPUT: &str = "controller_output";

/// 跨连续 run 缓冲的 chunk 类型。
pub const ACCUMULATING_TYPES: &[&str] = &[CHUNK_LLM_OUTPUT, CHUNK_LLM_REASONING];

/// `run_agent_team_streaming` 产出的首个 message chunk 事件类型。
pub const RUNTIME_READY_EVENT: &str = "team.runtime_ready";

/// 可读性上限:模型文本输出从不截断,体积大的工具载荷截断。
pub const TOOL_RESULT_CAP: usize = 2000;
pub const TOOL_ARGS_CAP: usize = 500;
pub const GENERIC_CAP: usize = 2000;

pub const UNKNOWN: &str = "<unknown>";

/// 日志类别 → 级别标签(对齐 `_CATEGORY_LEVEL`)。
pub fn category_level(category: &str) -> &'static str {
    match category {
        "text" => "INFO",
        "reasoning" => "DEBUG",
        "tool_call" => "DEBUG",
        "tool_result" => "DEBUG",
        "tool_update" => "DEBUG",
        "interaction" => "WARN",
        "controller_output" => "WARN",
        "runtime_ready" => "INFO",
        "message" => "INFO",
        "todo" => "INFO",
        _ => "INFO",
    }
}

/// 截断到 limit 字符并带可见标记(对齐 `_cap`)。
pub fn cap(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let truncated: String = text.chars().take(limit).collect();
    format!("{truncated}… (truncated)")
}

/// 从 chunk payload 提取文本内容(对齐 `_extract_content`)。
pub fn extract_content(payload: &Value) -> String {
    if let Some(obj) = payload.as_object() {
        if let Some(content) = obj.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            return content.to_string();
        }
        if let Some(output) = obj.get("output").and_then(Value::as_str) {
            return output.to_string();
        }
        return String::new();
    }
    if let Some(text) = payload.as_str() {
        return text.to_string();
    }
    payload.to_string()
}

/// chunk type + payload → 日志类别(对齐 `_classify`)。
pub fn classify(ctype: &str, payload: &Value) -> &'static str {
    if ctype == CHUNK_LLM_OUTPUT || ctype == CHUNK_ANSWER {
        return "text";
    }
    if ctype == CHUNK_LLM_REASONING {
        return "reasoning";
    }
    if ctype == CHUNK_TOOL_CALL {
        return "tool_call";
    }
    if ctype == CHUNK_TOOL_RESULT {
        return "tool_result";
    }
    if ctype == CHUNK_TOOL_UPDATE {
        return "tool_update";
    }
    if ctype == CHUNK_INTERACTION {
        return "interaction";
    }
    if ctype == CHUNK_CONTROLLER_OUTPUT {
        return "controller_output";
    }
    if ctype == CHUNK_MESSAGE {
        let is_ready = payload
            .as_object()
            .and_then(|obj| obj.get("event_type"))
            .and_then(Value::as_str)
            .map(|e| e == RUNTIME_READY_EVENT)
            .unwrap_or(false);
        return if is_ready { "runtime_ready" } else { "message" };
    }
    if ctype == CHUNK_TODO_UPDATED {
        return "todo";
    }
    "other"
}

/// `tool_call` chunk 单行摘要(对齐 `_tool_call_summary`)。
pub fn tool_call_summary(payload: &Value) -> String {
    let Some(obj) = payload.as_object() else {
        return cap(&payload.to_string(), GENERIC_CAP);
    };
    let name = obj.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let args_raw = obj.get("tool_args").map(str_value).unwrap_or_default();
    if name.is_empty() && args_raw.is_empty() {
        return cap(&payload.to_string(), GENERIC_CAP);
    }
    let args = cap(&args_raw, TOOL_ARGS_CAP);
    format!("tool_name={name} tool_args={args}")
}

/// `tool_result` chunk 两行摘要(对齐 `_tool_result_summary`)。
pub fn tool_result_summary(payload: &Value) -> String {
    let Some(obj) = payload.as_object() else {
        return cap(&payload.to_string(), GENERIC_CAP);
    };
    let name = obj.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let args_raw = obj.get("tool_args").map(str_value).unwrap_or_default();
    let result_raw = obj.get("tool_result").map(str_value).unwrap_or_default();
    if name.is_empty() && args_raw.is_empty() && result_raw.is_empty() {
        return cap(&payload.to_string(), GENERIC_CAP);
    }
    let args = cap(&args_raw, TOOL_ARGS_CAP);
    let result = cap(&result_raw, TOOL_RESULT_CAP);
    format!("tool_name={name} tool_args={args}\nresult: {result}")
}

/// `tool_update` chunk 摘要(对齐 `_tool_update_summary`)。
pub fn tool_update_summary(payload: &Value) -> String {
    let Some(obj) = payload.as_object() else {
        return cap(&payload.to_string(), GENERIC_CAP);
    };
    let Some(update) = obj.get("tool_update").and_then(Value::as_object) else {
        return cap(&payload.to_string(), GENERIC_CAP);
    };
    let name = update
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = update.get("status").and_then(Value::as_str).unwrap_or("");
    let call_id = update
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args_raw = update.get("arguments").map(str_value).unwrap_or_default();
    let args = cap(&args_raw, TOOL_ARGS_CAP);
    format!("tool_name={name} status={status} tool_call_id={call_id} arguments={args}")
}

/// 值 → Python `str()` 语义:字符串原样,其余 JSON 序列化。
fn str_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `controller_output` 可读任务失败消息(对齐 `_controller_output_summary`)。
pub fn controller_output_summary(payload: &Value) -> String {
    if let Some(obj) = payload.as_object() {
        let payload_type = obj
            .get("type")
            .map(Value::to_string)
            .unwrap_or_default()
            .to_lowercase();
        if payload_type.contains("task_failed")
            && let Some(data) = obj.get("data").and_then(Value::as_array)
        {
            let texts: Vec<String> = data
                .iter()
                .filter_map(|item| {
                    item.as_object()
                        .and_then(|o| o.get("text"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                })
                .collect();
            if !texts.is_empty() {
                return texts.join("\n");
            }
        }
    }
    cap(&payload.to_string(), GENERIC_CAP)
}

/// `team.runtime_ready` ack 紧凑摘要(对齐 `_runtime_ready_summary`)。
pub fn runtime_ready_summary(payload: &Value) -> String {
    let Some(obj) = payload.as_object() else {
        return cap(&payload.to_string(), GENERIC_CAP);
    };
    let team = obj.get("team_name").map(str_value).unwrap_or_default();
    let session = obj.get("session_id").map(str_value).unwrap_or_default();
    let activation = obj
        .get("activation_kind")
        .map(str_value)
        .unwrap_or_default();
    format!("team={team} session={session} activation={activation}")
}

/// `__interaction__`(HITL 请求)摘要(对齐 `_interaction_summary`)。
pub fn interaction_summary(payload: &Value) -> String {
    let iid = payload
        .as_object()
        .and_then(|obj| obj.get("interaction_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    format!(
        "interaction_id={iid}\n{}",
        cap(&payload.to_string(), GENERIC_CAP)
    )
}

/// `message`/`todo`/未知 chunk 尽力内容(对齐 `_generic_summary`)。
pub fn generic_summary(payload: &Value) -> String {
    let content = extract_content(payload);
    if !content.is_empty() {
        return content;
    }
    cap(&payload.to_string(), GENERIC_CAP)
}

/// 离散 chunk 摘要路由(对齐 `_discrete_summary`)。
pub fn discrete_summary(category: &str, payload: &Value) -> String {
    match category {
        "tool_call" => tool_call_summary(payload),
        "tool_result" => tool_result_summary(payload),
        "tool_update" => tool_update_summary(payload),
        "controller_output" => controller_output_summary(payload),
        "runtime_ready" => runtime_ready_summary(payload),
        "interaction" => interaction_summary(payload),
        _ => generic_summary(payload),
    }
}

/// 每个 (member, role) 来源的待写 run。
#[derive(Debug, Clone)]
struct Run {
    category: String,
    buf: Vec<String>,
}

/// 来源键:(member, role)。
type SourceKey = (Option<String>, Option<String>);

/// 聚合团队流式诊断日志(对齐 `TeamStreamLogger`)。
pub struct TeamStreamLogger {
    path: PathBuf,
    file: Mutex<Option<std::fs::File>>,
    runs: Mutex<HashMap<SourceKey, Run>>,
    llm_output_seen: Mutex<HashSet<SourceKey>>,
    chunk_count: Mutex<u64>,
}

/// 输入 chunk 视图:未打团队标签的 chunk(infra 层透传,Plain)直接跳过,
/// 团队 chunk(Team)按流日志语义处理 —— 对齐 Python
/// `if not isinstance(chunk, TeamOutputSchema): return`。
pub enum StreamChunk<'a> {
    /// 团队输出 chunk(带 source_member/role 标记)。
    Team(&'a TeamOutputSchema),
    /// 普通输出 chunk(未标记,跳过)。
    Plain(&'a ah_contracts::stream::OutputSchema),
}

impl TeamStreamLogger {
    /// 打开 *file_path* 追加聚合流式记录;父目录缺失则创建;路径不可用显式失败。
    pub fn new(file_path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;
        Ok(Self {
            path: file_path.to_path_buf(),
            file: Mutex::new(Some(file)),
            runs: Mutex::new(HashMap::new()),
            llm_output_seen: Mutex::new(HashSet::new()),
            chunk_count: Mutex::new(0),
        })
    }

    /// 已打开的文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 消费一个流 chunk:缓冲或立即写入。永不抛错。
    pub fn feed(&self, chunk: StreamChunk<'_>) {
        match self.feed_inner(chunk) {
            Ok(()) => {}
            Err(err) => {
                let body = format!("[WARN] stream logger feed error: {err:?}");
                self.safe_write(&body);
            }
        }
    }

    /// 冲刷所有待写 run 并关闭文件。永不抛错。
    pub fn flush(&self) {
        if let Err(err) = self.flush_inner() {
            let body = format!("[WARN] stream logger flush error: {err:?}");
            self.safe_write(&body);
        }
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    fn feed_inner(&self, chunk: StreamChunk<'_>) -> std::io::Result<()> {
        // 对齐 Python `_feed`:`_chunk_count += 1` 在 isinstance 检查之前。
        *self.chunk_count.lock().unwrap() += 1;
        // 未打团队标签的 chunk(infra 层透传)跳过。
        let team = match chunk {
            StreamChunk::Team(team) => team,
            StreamChunk::Plain(_) => return Ok(()),
        };
        self.feed_team(team)
    }

    fn feed_team(&self, chunk: &TeamOutputSchema) -> std::io::Result<()> {
        let ctype = chunk.r#type.as_str();
        let payload = &chunk.payload;
        let member = chunk.source_member.clone();
        let role = chunk.role.clone();
        let key = (member, role);

        // `answer` 与同一来源已出现的 llm_output 去重。
        if ctype == CHUNK_ANSWER && self.llm_output_seen.lock().unwrap().contains(&key) {
            return Ok(());
        }

        let category = classify(ctype, payload);

        if ACCUMULATING_TYPES.contains(&ctype) {
            let content = extract_content(payload);
            if content.is_empty() {
                return Ok(());
            }
            let mut runs = self.runs.lock().unwrap();
            let mut run = runs.get(&key).cloned();
            if let Some(existing) = &run
                && existing.category != category
            {
                // 同来源切换类别 → 先冲刷旧 run。
                drop(runs);
                self.flush_key(&key)?;
                run = None;
                runs = self.runs.lock().unwrap();
            }
            let mut run = run.unwrap_or_else(|| Run {
                category: category.to_string(),
                buf: Vec::new(),
            });
            run.buf.push(content);
            if ctype == CHUNK_LLM_OUTPUT {
                self.llm_output_seen.lock().unwrap().insert(key.clone());
            }
            runs.insert(key, run);
            return Ok(());
        }

        // 离散 chunk:先冲刷该来源待写 run,再立即写。
        self.flush_key(&key)?;
        let summary = discrete_summary(category, payload);
        self.emit(category, &key.0, &key.1, &summary);
        Ok(())
    }

    fn flush_inner(&self) -> std::io::Result<()> {
        let keys: Vec<(Option<String>, Option<String>)> =
            self.runs.lock().unwrap().keys().cloned().collect();
        for key in keys {
            self.flush_key(&key)?;
        }
        let count = *self.chunk_count.lock().unwrap();
        if count > 0 {
            let body = format!("[INFO] stream end, {count} chunks");
            self.safe_write(&body);
        }
        // 关闭文件。
        let mut guard = self.file.lock().unwrap();
        if let Some(file) = guard.take() {
            drop(file); // 显式关闭。
        }
        Ok(())
    }

    fn flush_key(&self, key: &(Option<String>, Option<String>)) -> std::io::Result<()> {
        let run = self.runs.lock().unwrap().remove(key);
        let Some(run) = run else {
            return Ok(());
        };
        if run.buf.is_empty() {
            return Ok(());
        }
        let content = run.buf.join("");
        self.emit(&run.category, &key.0, &key.1, &content);
        Ok(())
    }

    fn emit(&self, category: &str, member: &Option<String>, role: &Option<String>, content: &str) {
        if content.is_empty() {
            return;
        }
        let level = category_level(category);
        let member_s = member.as_deref().unwrap_or(UNKNOWN);
        let role_s = role.as_deref().unwrap_or(UNKNOWN);
        let prefixed = content
            .lines()
            .map(|line| format!("  | {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let header = format!("[{level}] member={member_s} role={role_s} category={category}");
        let body = format!("{header}\n{prefixed}");
        self.safe_write(&body);
    }

    /// 写一条时间戳记录;吞掉任何 IO 错误。
    fn safe_write(&self, body: &str) {
        let mut guard = self.file.lock().unwrap();
        let Some(file) = guard.as_mut() else {
            return;
        };
        let timestamp = now_local_millis();
        let line = format!("{timestamp} {body}\n");
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        let _ = file.flush();
    }
}

/// 本地时间戳(YYYY-MM-DD HH:MM:SS.mmm;对齐 Python
/// `datetime.now().astimezone().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]`)。
fn now_local_millis() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    format_local_millis(now, system_local_offset_secs())
}

/// 纯格式化:epoch 毫秒 + 本地偏移秒 → "YYYY-MM-DD HH:MM:SS.mmm"。
fn format_local_millis(epoch_ms: i64, offset_secs: i32) -> String {
    let local_secs = epoch_ms.div_euclid(1000) + i64::from(offset_secs);
    let millis = epoch_ms.rem_euclid(1000);
    let days = local_secs.div_euclid(86_400);
    let secs_of_day = local_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.{millis:03}")
}

/// days(自 1970-01-01)→ (year, month, day)(Howard Hinnant 算法)。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 系统本地时区偏移(秒,东为正):`date +%z` → `TZ` POSIX → UTC(0)。
fn system_local_offset_secs() -> i32 {
    if let Some(out) = std::process::Command::new("date").arg("+%z").output().ok()
        && out.status.success()
        && let Ok(text) = String::from_utf8(out.stdout)
        && let Some(secs) = parse_iso_offset(text.trim())
    {
        return secs;
    }
    if let Ok(tz) = std::env::var("TZ")
        && !tz.trim().is_empty()
        && !tz.trim_start().starts_with(':')
        && let Some(secs) = posix_offset_secs(tz.trim())
    {
        return secs;
    }
    0
}

/// "+HHMM"/"-HHMM" → 秒(东为正)。
fn parse_iso_offset(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 || (bytes[0] != b'+' && bytes[0] != b'-') {
        return None;
    }
    let hh = text.get(1..3)?.parse::<i32>().ok()?;
    let mm = text.get(3..5)?.parse::<i32>().ok()?;
    let secs = hh * 3600 + mm * 60;
    Some(if bytes[0] == b'-' { -secs } else { secs })
}

/// POSIX TZ 偏移解析(`std offset` 或纯 `offset`):东为正。
fn posix_offset_secs(tz: &str) -> Option<i32> {
    // 取最后一个空白分隔的 token(如 "CST-8" 的 "-8")。
    let token = tz.split_whitespace().last()?;
    if token.starts_with(':') || token.is_empty() {
        return None;
    }
    let digits_start = token.find(|c: char| c == '+' || c == '-' || c.is_ascii_digit())?;
    let offset_part = &token[digits_start..];
    if offset_part.is_empty() {
        return None;
    }
    let (sign, num_part) = if let Some(rest) = offset_part.strip_prefix('-') {
        (-1, rest)
    } else if let Some(rest) = offset_part.strip_prefix('+') {
        (1, rest)
    } else {
        (1, offset_part)
    };
    if num_part.len() < 2 || num_part.len() > 4 {
        return None;
    }
    let (hh, mm) = if num_part.len() == 2 {
        (num_part.parse::<i32>().ok()?, 0)
    } else if num_part.len() == 3 {
        (
            num_part.get(0..1)?.parse::<i32>().ok()?,
            num_part.get(1..3)?.parse::<i32>().ok()?,
        )
    } else {
        (
            num_part.get(0..2)?.parse::<i32>().ok()?,
            num_part.get(2..4)?.parse::<i32>().ok()?,
        )
    };
    if hh > 24 || mm > 59 {
        return None;
    }
    Some(sign * (hh * 3600 + mm * 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::stream::TeamOutputSchema;
    use serde_json::json;
    use std::io::Read;

    fn chunk(
        ctype: &str,
        payload: Value,
        member: Option<&str>,
        role: Option<&str>,
    ) -> TeamOutputSchema {
        TeamOutputSchema {
            r#type: ctype.to_string(),
            index: 0,
            payload,
            source_member: member.map(str::to_string),
            role: role.map(str::to_string),
        }
    }

    #[test]
    fn caps_and_classifies() {
        assert_eq!(cap("abc", 5), "abc");
        assert!(cap("abcdef", 3).ends_with("… (truncated)"));
        assert_eq!(classify(CHUNK_LLM_OUTPUT, &json!("x")), "text");
        assert_eq!(classify(CHUNK_ANSWER, &json!("x")), "text");
        assert_eq!(classify(CHUNK_LLM_REASONING, &json!("x")), "reasoning");
        assert_eq!(classify(CHUNK_TOOL_CALL, &json!({})), "tool_call");
        assert_eq!(classify(CHUNK_TOOL_RESULT, &json!({})), "tool_result");
        assert_eq!(classify(CHUNK_TOOL_UPDATE, &json!({})), "tool_update");
        assert_eq!(classify(CHUNK_INTERACTION, &json!({})), "interaction");
        assert_eq!(
            classify(CHUNK_CONTROLLER_OUTPUT, &json!({})),
            "controller_output"
        );
        assert_eq!(classify(CHUNK_TODO_UPDATED, &json!({})), "todo");
        assert_eq!(
            classify(CHUNK_MESSAGE, &json!({"event_type": "team.runtime_ready"})),
            "runtime_ready"
        );
        assert_eq!(
            classify(CHUNK_MESSAGE, &json!({"event_type": "other"})),
            "message"
        );
        assert_eq!(classify("weird", &json!({})), "other");
        assert_eq!(category_level("text"), "INFO");
        assert_eq!(category_level("reasoning"), "DEBUG");
        assert_eq!(category_level("interaction"), "WARN");
        assert_eq!(category_level("controller_output"), "WARN");
    }

    #[test]
    fn summaries_match_python_shapes() {
        assert_eq!(
            tool_call_summary(&json!({"tool_name": "bash", "tool_args": "ls"})),
            "tool_name=bash tool_args=ls"
        );
        assert_eq!(
            tool_result_summary(
                &json!({"tool_name": "bash", "tool_args": "ls", "tool_result": "ok"})
            ),
            "tool_name=bash tool_args=ls\nresult: ok"
        );
        // 缺标准键 → 整包摘要兜底。
        assert!(tool_call_summary(&json!({"weird": 1})).contains("weird"));
        // tool_update 包装键。
        assert_eq!(
            tool_update_summary(&json!({
                "tool_update": {"tool_name": "bash", "status": "in_progress", "tool_call_id": "c1", "arguments": "ls"}
            })),
            "tool_name=bash status=in_progress tool_call_id=c1 arguments=ls"
        );
        // controller_output task_failed 提取文本。
        assert_eq!(
            controller_output_summary(&json!({
                "type": "task_failed",
                "data": [{"text": "  boom  "}, {"text": ""}, {"text": "second"}]
            })),
            "boom\nsecond"
        );
        assert_eq!(
            runtime_ready_summary(&json!({
                "team_name": "alpha", "session_id": "s1", "activation_kind": "cold"
            })),
            "team=alpha session=s1 activation=cold"
        );
        assert!(
            interaction_summary(&json!({"interaction_id": "i9"})).starts_with("interaction_id=i9")
        );
        assert_eq!(generic_summary(&json!({"content": "hello"})), "hello");
        assert_eq!(extract_content(&json!("plain")), "plain");
        assert_eq!(extract_content(&json!({"output": "out"})), "out");
    }

    #[test]
    fn format_local_millis_shape() {
        // 已知值:1577931845 s → 2020-01-02 02:24:05 UTC。
        let text = format_local_millis(1_577_931_845_678, 0);
        assert_eq!(text, "2020-01-02 02:24:05.678");
    }

    #[test]
    fn logger_buffers_and_flushes_runs() {
        let dir = std::env::temp_dir().join(format!("ah-sl-{}", std::process::id()));
        let path = dir.join("team.log");
        let logger = TeamStreamLogger::new(&path).expect("open");

        // leader 文本 run。
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_LLM_OUTPUT,
            json!("hello "),
            Some("leader"),
            Some("leader"),
        )));
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_LLM_OUTPUT,
            json!("world"),
            Some("leader"),
            Some("leader"),
        )));
        // teammate 推理穿插 → 各自独立 run。
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_LLM_REASONING,
            json!("think "),
            Some("tm1"),
            Some("teammate"),
        )));
        // leader 离散 tool_call 冲刷其待写 run。
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_TOOL_CALL,
            json!({"tool_name": "bash", "tool_args": "ls"}),
            Some("leader"),
            Some("leader"),
        )));
        // answer 去重:leader 已出 llm_output → 跳过。
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_ANSWER,
            json!("final"),
            Some("leader"),
            Some("leader"),
        )));
        // 未标记 chunk(infra)→ 跳过。
        let plain = ah_contracts::stream::OutputSchema::new("llm_output", 0, json!("skip"));
        logger.feed(StreamChunk::Plain(&plain));

        logger.flush();

        let mut text = String::new();
        std::fs::File::open(&path)
            .expect("open file")
            .read_to_string(&mut text)
            .expect("read");
        // leader text run 合并为一条记录。
        assert!(
            text.contains("member=leader role=leader category=text"),
            "text: {text}"
        );
        assert!(text.contains("  | hello world"), "merged run: {text}");
        // teammate reasoning 独立记录。
        assert!(text.contains("member=tm1 role=teammate category=reasoning"));
        assert!(text.contains("  | think "));
        // 离散 tool_call 记录。
        assert!(text.contains("member=leader role=leader category=tool_call"));
        assert!(text.contains("tool_name=bash tool_args=ls"));
        // answer 去重:无 "final"。
        assert!(!text.contains("final"), "answer dedup: {text}");
        // infra chunk 跳过:无 "skip"。
        assert!(!text.contains("skip"));
        // 流结束摘要。
        assert!(
            text.contains("[INFO] stream end, 6 chunks"),
            "end marker: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logger_category_switch_flushes_old_run() {
        let dir = std::env::temp_dir().join(format!("ah-sl2-{}", std::process::id()));
        let path = dir.join("team.log");
        let logger = TeamStreamLogger::new(&path).expect("open");

        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_LLM_OUTPUT,
            json!("text-a "),
            Some("leader"),
            Some("leader"),
        )));
        // 同来源切到 reasoning → 冲刷 text run。
        logger.feed(StreamChunk::Team(&chunk(
            CHUNK_LLM_REASONING,
            json!("think-b"),
            Some("leader"),
            Some("leader"),
        )));
        logger.flush();

        let mut text = String::new();
        std::fs::File::open(&path)
            .expect("open")
            .read_to_string(&mut text)
            .expect("read");
        assert!(text.contains("  | text-a "), "old run flushed: {text}");
        assert!(text.contains("  | think-b"), "new run written: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
