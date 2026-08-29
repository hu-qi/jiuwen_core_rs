//! # ah-plugins-common-tools
//!
//! 常用 coding-agent 工具(P2-01):`todo` 与 `cron`,文件持久化、结构化输出、
//! 显式错误。两者都只依赖 tools seam(注入 TOOLS,注册工具实例),不提供业务 seam。
//!
//! - **todo**:按 session 隔离的待办列表(JSON 文件),add/update/remove/list;
//! - **cron**:文件持久化的定时任务注册表,add/list/remove/toggle +
//!   确定性 `schedule` 解析(cron 五字段子集)与 `next_run_after` 计算
//!   (调度执行循环属运行时组件,不在本工具层,后续接入)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::keys::TOOLS;
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// todo
// ---------------------------------------------------------------------------

/// 待办项(文件持久化,JSON)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String, // "pending" | "in_progress" | "done"
}

/// 按 session 隔离的待办存储:`<root>/todos/<session>.json`。
#[derive(Default)]
pub struct TodoStore {
    dir: PathBuf,
}

impl TodoStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, session_id: &str) -> PathBuf {
        let safe: String = session_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    fn load(&self, session_id: &str) -> Vec<TodoItem> {
        let path = self.path(session_id);
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<TodoItem>>(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, session_id: &str, items: &[TodoItem]) -> Result<(), ToolError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| ToolError(format!("todo dir create failed: {e}")))?;
        let path = self.path(session_id);
        let text = serde_json::to_string_pretty(items)
            .map_err(|e| ToolError(format!("todo serialize failed: {e}")))?;
        std::fs::write(path, text).map_err(|e| ToolError(format!("todo write failed: {e}")))
    }
}

/// todo 工具:add / update / remove / list(按 session 隔离,文件持久化)。
pub struct TodoTool {
    store: TodoStore,
}

impl TodoTool {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            store: TodoStore::new(dir),
        }
    }

    fn next_id(items: &[TodoItem]) -> String {
        let max = items
            .iter()
            .filter_map(|item| item.id.trim_start_matches('t').parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("t{}", max + 1)
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &'static str {
        "todo"
    }

    fn description(&self) -> &'static str {
        "manage a session-scoped todo list (add/update/remove/list)"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "update", "remove", "list"] },
                "session_id": { "type": "string" },
                "id": { "type": "string" },
                "content": { "type": "string" },
                "status": { "type": "string", "enum": ["pending", "in_progress", "done"] },
            },
            "required": ["action"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field action".to_string()))?;
        let session_id = arguments
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let mut items = self.store.load(session_id);
        match action {
            "add" => {
                let content = arguments
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|c| !c.trim().is_empty())
                    .ok_or_else(|| {
                        ToolError("missing non-empty string field content".to_string())
                    })?;
                let id = Self::next_id(&items);
                let item = TodoItem {
                    id: id.clone(),
                    content: content.to_string(),
                    status: arguments
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("pending")
                        .to_string(),
                };
                items.push(item);
                self.store.save(session_id, &items)?;
                Ok(json!({ "action": "add", "id": id, "count": items.len() }))
            }
            "update" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field id".to_string()))?;
                let item = items
                    .iter_mut()
                    .find(|item| item.id == id)
                    .ok_or_else(|| ToolError(format!("todo id not found: {id}")))?;
                if let Some(content) = arguments.get("content").and_then(Value::as_str) {
                    item.content = content.to_string();
                }
                if let Some(status) = arguments.get("status").and_then(Value::as_str) {
                    item.status = status.to_string();
                }
                self.store.save(session_id, &items)?;
                Ok(json!({ "action": "update", "id": id, "count": items.len() }))
            }
            "remove" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field id".to_string()))?;
                let before = items.len();
                items.retain(|item| item.id != id);
                if items.len() == before {
                    return Err(ToolError(format!("todo id not found: {id}")));
                }
                self.store.save(session_id, &items)?;
                Ok(json!({ "action": "remove", "id": id, "count": items.len() }))
            }
            "list" => Ok(json!({
                "action": "list",
                "session_id": session_id,
                "count": items.len(),
                "todos": items,
            })),
            other => Err(ToolError(format!("unknown todo action: {other}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// cron
// ---------------------------------------------------------------------------

/// cron 五字段表达式(子集):分 时 日 月 周;每字段支持 `*` / `N` / `*/N`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub minute: String,
    pub hour: String,
    pub day: String,
    pub month: String,
    pub weekday: String,
    pub raw: String,
}

/// UTC 历法换算(纯 std,确定性;Hinnant days_from_civil/civil_from_days)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl UtcTime {
    pub fn from_epoch_secs(secs: i64) -> Self {
        let days = secs.div_euclid(86400);
        let rem = secs.rem_euclid(86400);
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour: (rem / 3600) as u32,
            minute: ((rem % 3600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }

    pub fn to_epoch_secs(&self) -> i64 {
        let days = days_from_civil(self.year, self.month, self.day);
        days * 86400 + (self.hour as i64) * 3600 + (self.minute as i64) * 60 + self.second as i64
    }

    /// 周日=0 … 周六=6。
    pub fn weekday(&self) -> u32 {
        let days = days_from_civil(self.year, self.month, self.day);
        // 1970-01-01 是周四(4);(days + 4) mod 7
        (days + 4).rem_euclid(7) as u32
    }
}

fn field_match(spec: &str, value: u32) -> bool {
    let spec = spec.trim();
    if spec == "*" {
        return true;
    }
    if let Some(stripped) = spec.strip_prefix("*/") {
        return stripped
            .parse::<u32>()
            .ok()
            .map(|step| step > 0 && value.is_multiple_of(step))
            .unwrap_or(false);
    }
    spec.parse::<u32>().map(|v| v == value).unwrap_or(false)
}

impl Schedule {
    /// 解析 cron 五字段子集(分 时 日 月 周);非法字段显式报错。
    pub fn parse(raw: &str) -> Result<Self, String> {
        let fields: Vec<&str> = raw.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "cron expression must have 5 fields (minute hour day month weekday), got {}: {raw}",
                fields.len()
            ));
        }
        let validate = |spec: &str, max: u32, name: &str| -> Result<(), String> {
            if spec == "*" {
                return Ok(());
            }
            if let Some(stripped) = spec.strip_prefix("*/") {
                let step: u32 = stripped
                    .parse()
                    .map_err(|_| format!("invalid {name} step: {spec}"))?;
                if step == 0 || step > max {
                    return Err(format!("invalid {name} step: {spec}"));
                }
                return Ok(());
            }
            let value: u32 = spec
                .parse()
                .map_err(|_| format!("invalid {name} field: {spec}"))?;
            if value > max {
                return Err(format!("{name} value out of range: {spec}"));
            }
            Ok(())
        };
        validate(fields[0], 59, "minute")?;
        validate(fields[1], 23, "hour")?;
        validate(fields[2], 31, "day")?;
        validate(fields[3], 12, "month")?;
        validate(fields[4], 6, "weekday")?;
        Ok(Self {
            minute: fields[0].to_string(),
            hour: fields[1].to_string(),
            day: fields[2].to_string(),
            month: fields[3].to_string(),
            weekday: fields[4].to_string(),
            raw: raw.to_string(),
        })
    }

    pub fn matches(&self, t: &UtcTime) -> bool {
        field_match(&self.minute, t.minute)
            && field_match(&self.hour, t.hour)
            && field_match(&self.day, t.day)
            && field_match(&self.month, t.month)
            && field_match(&self.weekday, t.weekday())
    }

    /// 从 `from_epoch_secs` 之后(严格大于)起,24 小时内下一次命中的 epoch 秒。
    pub fn next_run_after(&self, from_epoch_secs: i64) -> Option<i64> {
        // 24h 内按分钟扫描(1440 步),确定性。
        let mut cursor = from_epoch_secs - from_epoch_secs.rem_euclid(60) + 60;
        let horizon = from_epoch_secs + 24 * 3600;
        while cursor <= horizon {
            if self.matches(&UtcTime::from_epoch_secs(cursor)) {
                return Some(cursor);
            }
            cursor += 60;
        }
        None
    }
}

/// 定时任务(文件持久化,JSON)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub command: String,
    pub schedule: String,
    pub enabled: bool,
    pub created_at: i64,
}

/// cron 存储:`<root>/cron/jobs.json`。
pub struct CronStore {
    path: PathBuf,
}

impl CronStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            path: dir.join("jobs.json"),
        }
    }

    fn load(&self) -> Vec<CronJob> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<CronJob>>(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, jobs: &[CronJob]) -> Result<(), ToolError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ToolError(format!("cron dir create failed: {e}")))?;
        }
        let text = serde_json::to_string_pretty(jobs)
            .map_err(|e| ToolError(format!("cron serialize failed: {e}")))?;
        std::fs::write(&self.path, text).map_err(|e| ToolError(format!("cron write failed: {e}")))
    }
}

/// cron 工具:add / list / remove / toggle + `next_run` 查询。
pub struct CronTool {
    store: CronStore,
}

impl CronTool {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            store: CronStore::new(dir),
        }
    }

    fn next_id(jobs: &[CronJob]) -> String {
        let max = jobs
            .iter()
            .filter_map(|job| job.id.trim_start_matches('j').parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("j{}", max + 1)
    }
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &'static str {
        "cron"
    }

    fn description(&self) -> &'static str {
        "manage scheduled jobs (add/list/remove/toggle/next_run), cron 5-field subset"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "list", "remove", "toggle", "next_run"] },
                "id": { "type": "string" },
                "command": { "type": "string" },
                "schedule": { "type": "string" },
                "enabled": { "type": "boolean" },
            },
            "required": ["action"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field action".to_string()))?;
        let mut jobs = self.store.load();
        match action {
            "add" => {
                let command = arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|c| !c.trim().is_empty())
                    .ok_or_else(|| {
                        ToolError("missing non-empty string field command".to_string())
                    })?;
                let schedule = arguments
                    .get("schedule")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field schedule".to_string()))?;
                Schedule::parse(schedule)
                    .map_err(|e| ToolError(format!("invalid schedule: {e}")))?;
                let id = Self::next_id(&jobs);
                let job = CronJob {
                    id: id.clone(),
                    command: command.to_string(),
                    schedule: schedule.to_string(),
                    enabled: arguments
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    created_at: now_epoch(),
                };
                jobs.push(job);
                self.store.save(&jobs)?;
                Ok(json!({ "action": "add", "id": id, "count": jobs.len() }))
            }
            "list" => Ok(json!({ "action": "list", "count": jobs.len(), "jobs": jobs })),
            "remove" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field id".to_string()))?;
                let before = jobs.len();
                jobs.retain(|job| job.id != id);
                if jobs.len() == before {
                    return Err(ToolError(format!("cron job id not found: {id}")));
                }
                self.store.save(&jobs)?;
                Ok(json!({ "action": "remove", "id": id, "count": jobs.len() }))
            }
            "toggle" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field id".to_string()))?;
                let enabled = {
                    let job = jobs
                        .iter_mut()
                        .find(|job| job.id == id)
                        .ok_or_else(|| ToolError(format!("cron job id not found: {id}")))?;
                    job.enabled = arguments
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(!job.enabled);
                    job.enabled
                };
                self.store.save(&jobs)?;
                Ok(json!({ "action": "toggle", "id": id, "enabled": enabled }))
            }
            "next_run" => {
                let id = arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ToolError("missing string field id".to_string()))?;
                let job = jobs
                    .iter()
                    .find(|job| job.id == id)
                    .ok_or_else(|| ToolError(format!("cron job id not found: {id}")))?;
                let schedule = Schedule::parse(&job.schedule)
                    .map_err(|e| ToolError(format!("invalid stored schedule: {e}")))?;
                let next = schedule.next_run_after(now_epoch());
                Ok(json!({ "action": "next_run", "id": id, "next_epoch": next }))
            }
            other => Err(ToolError(format!("unknown cron action: {other}"))),
        }
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// plugin
// ---------------------------------------------------------------------------

/// 注册 todo/cron 工具到 tools seam(依赖 TOOLS;持久化目录由宿主注入)。
pub struct CommonToolsPlugin {
    root: PathBuf,
}

impl CommonToolsPlugin {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Plugin for CommonToolsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-common-tools"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        let effects = vec![
            registry.register(Arc::new(TodoTool::new(self.root.join("todos")))),
            registry.register(Arc::new(CronTool::new(self.root.join("cron")))),
        ];
        Ok(effects)
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn utc_roundtrip_is_exact() {
        // 已知向量:2020-02-29 12:34:56 UTC
        let t = UtcTime {
            year: 2020,
            month: 2,
            day: 29,
            hour: 12,
            minute: 34,
            second: 56,
        };
        let secs = t.to_epoch_secs();
        assert_eq!(secs, 1582979696);
        assert_eq!(UtcTime::from_epoch_secs(secs), t);
        // 1970-01-01 00:00:00 UTC = 0,周四(weekday=4)
        let epoch = UtcTime::from_epoch_secs(0);
        assert_eq!(
            epoch,
            UtcTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        assert_eq!(epoch.weekday(), 4);
    }

    #[test]
    fn schedule_parse_rejects_bad_fields() {
        assert!(Schedule::parse("a * * * *").is_err());
        assert!(Schedule::parse("* * * * * *").is_err());
        assert!(Schedule::parse("*/0 * * * *").is_err());
        assert!(Schedule::parse("60 * * * *").is_err());
        assert!(Schedule::parse("*/5 * * * *").is_ok());
    }

    #[test]
    fn schedule_next_run_is_deterministic() {
        // 每 5 分钟;基准 2020-02-29 12:34:56 → 下一命中 12:35:00
        let s = Schedule::parse("*/5 * * * *").unwrap();
        let base = UtcTime {
            year: 2020,
            month: 2,
            day: 29,
            hour: 12,
            minute: 34,
            second: 56,
        }
        .to_epoch_secs();
        let next = s.next_run_after(base).expect("next run");
        let expected = UtcTime {
            year: 2020,
            month: 2,
            day: 29,
            hour: 12,
            minute: 35,
            second: 0,
        }
        .to_epoch_secs();
        assert_eq!(next, expected);
        // 每分钟调度:下一分钟整点。
        let every_min = Schedule::parse("* * * * *").unwrap();
        assert_eq!(
            every_min.next_run_after(base).unwrap(),
            base + 60 - base.rem_euclid(60)
        );
    }

    #[tokio::test]
    async fn todo_crud_is_file_backed_and_structured() {
        let dir = std::env::temp_dir().join(format!("ah-todo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tool = TodoTool::new(dir.clone());
        let added = tool
            .invoke(json!({ "action": "add", "session_id": "s1", "content": "write tests" }))
            .await
            .expect("add");
        assert_eq!(added["action"], "add");
        let id = added["id"].as_str().unwrap().to_string();
        let _ = tool
            .invoke(json!({ "action": "update", "session_id": "s1", "id": id, "status": "done" }))
            .await
            .expect("update");
        let listed = tool
            .invoke(json!({ "action": "list", "session_id": "s1" }))
            .await
            .expect("list");
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["todos"][0]["status"], "done");
        // 重开持久化:新建 store 读同一文件。
        let reopened = TodoTool::new(dir.clone());
        let relisted = reopened
            .invoke(json!({ "action": "list", "session_id": "s1" }))
            .await
            .expect("relist");
        assert_eq!(relisted["count"], 1, "文件持久化");
        // 错误:未知 id / 缺 content。
        assert!(
            tool.invoke(json!({ "action": "update", "session_id": "s1", "id": "t99" }))
                .await
                .is_err()
        );
        assert!(
            tool.invoke(json!({ "action": "add", "session_id": "s1" }))
                .await
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cron_crud_validates_schedule_and_persists() {
        let dir = std::env::temp_dir().join(format!("ah-cron-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let tool = CronTool::new(dir.clone());
        // 非法 schedule 显式拒绝。
        assert!(
            tool.invoke(json!({ "action": "add", "command": "x", "schedule": "not a cron" }))
                .await
                .is_err()
        );
        let added = tool
            .invoke(json!({ "action": "add", "command": "echo hi", "schedule": "*/5 * * * *" }))
            .await
            .expect("add");
        let id = added["id"].as_str().unwrap().to_string();
        let listed = tool
            .invoke(json!({ "action": "list" }))
            .await
            .expect("list");
        assert_eq!(listed["count"], 1);
        // 持久化 + next_run。
        let reopened = CronTool::new(dir.clone());
        let next = reopened
            .invoke(json!({ "action": "next_run", "id": id }))
            .await
            .expect("next_run");
        assert!(next["next_epoch"].as_i64().unwrap() > 0);
        let _ = reopened
            .invoke(json!({ "action": "toggle", "id": id, "enabled": false }))
            .await
            .expect("toggle");
        let relisted = reopened
            .invoke(json!({ "action": "list" }))
            .await
            .expect("list");
        assert_eq!(relisted["jobs"][0]["enabled"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
