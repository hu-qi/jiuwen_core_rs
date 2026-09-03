//! # ah-plugins-controller
//!
//! 真实控制器(对齐 Python core/controller):
//! - TaskManager:任务 CRUD/状态迁移/优先级索引/父子层级(防环);
//! - TaskScheduler:执行器注册表 + 真实执行(消费方注入后端)+ 冲突处理
//!   (同会话已有 working 任务 → 拒绝新执行);
//! - IntentRecognizer:确定性关键词意图识别(create/pause/resume/cancel/
//!   switch/modify/continue/supplement,未知 → UnknownTask)。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ah_contracts::controller::{
    Controller, ControllerError, Intent, IntentType, Task, TaskExecutor, TaskFilter,
    TaskSnapshotStore, TaskStatus,
};
use ah_contracts::effect::Effect;
use ah_contracts::keys::{CONTROLLER, TASK_SNAPSHOT_STORE};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 可逆状态迁移表:from → 允许的 to。
fn allowed_transitions(from: TaskStatus) -> &'static [TaskStatus] {
    match from {
        TaskStatus::Submitted => &[
            TaskStatus::Working,
            TaskStatus::Canceled,
            TaskStatus::Waiting,
        ],
        TaskStatus::Working => &[
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Paused,
            TaskStatus::Canceled,
            TaskStatus::InputRequired,
        ],
        TaskStatus::Paused => &[TaskStatus::Submitted, TaskStatus::Canceled],
        TaskStatus::InputRequired => &[TaskStatus::Working, TaskStatus::Canceled],
        TaskStatus::Waiting => &[TaskStatus::Submitted, TaskStatus::Canceled],
        TaskStatus::Completed | TaskStatus::Canceled | TaskStatus::Failed | TaskStatus::Unknown => {
            &[]
        }
    }
}

/// 真实控制器。
pub struct InMemoryTaskSnapshotStore {
    tasks: Mutex<Vec<Task>>,
}

impl InMemoryTaskSnapshotStore {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
        }
    }
}

impl Default for InMemoryTaskSnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for InMemoryTaskSnapshotStore {}
impl TaskSnapshotStore for InMemoryTaskSnapshotStore {
    fn save(&self, tasks: &[Task]) -> Result<(), ControllerError> {
        *self.tasks.lock().unwrap() = tasks.to_vec();
        Ok(())
    }
    fn load(&self) -> Result<Vec<Task>, ControllerError> {
        Ok(self.tasks.lock().unwrap().clone())
    }
}

struct SnapshotFileLock {
    path: PathBuf,
}

impl Drop for SnapshotFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotEnvelope {
    version: u32,
    tasks: Vec<Task>,
}

const SNAPSHOT_VERSION: u32 = 1;

pub struct JsonTaskSnapshotStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonTaskSnapshotStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            write_lock: Mutex::new(()),
        }
    }
}

impl Seam for JsonTaskSnapshotStore {}
impl TaskSnapshotStore for JsonTaskSnapshotStore {
    fn save(&self, tasks: &[Task]) -> Result<(), ControllerError> {
        let _guard = self.write_lock.lock().unwrap();
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .map_err(|e| ControllerError(format!("snapshot directory: {e}")))?;
        let lock_path = self.path.with_extension("lock");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| ControllerError(format!("snapshot lock: {e}")))?;
        let _lock = SnapshotFileLock { path: lock_path };
        let temporary = self.path.with_extension("tmp");
        let envelope = SnapshotEnvelope {
            version: SNAPSHOT_VERSION,
            tasks: tasks.to_vec(),
        };
        let data = serde_json::to_vec_pretty(&envelope)
            .map_err(|e| ControllerError(format!("snapshot encode: {e}")))?;
        std::fs::write(&temporary, data)
            .map_err(|e| ControllerError(format!("snapshot write: {e}")))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|e| ControllerError(format!("snapshot replace: {e}")))
    }
    fn load(&self) -> Result<Vec<Task>, ControllerError> {
        match std::fs::read(&self.path) {
            Ok(data) => {
                let value: serde_json::Value = serde_json::from_slice(&data)
                    .map_err(|e| ControllerError(format!("snapshot decode: {e}")))?;
                if value.is_object() {
                    let snapshot: SnapshotEnvelope = serde_json::from_value(value)
                        .map_err(|e| ControllerError(format!("snapshot envelope: {e}")))?;
                    if snapshot.version != SNAPSHOT_VERSION {
                        return Err(ControllerError(format!(
                            "snapshot version unsupported: {}",
                            snapshot.version
                        )));
                    }
                    Ok(snapshot.tasks)
                } else {
                    serde_json::from_value(value)
                        .map_err(|e| ControllerError(format!("snapshot legacy array decode: {e}")))
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(ControllerError(format!("snapshot read: {error}"))),
        }
    }
}

pub struct LocalController {
    tasks: Mutex<HashMap<String, Task>>,
    /// task_id → children。
    parent_to_children: Mutex<HashMap<String, HashSet<String>>>,
    /// task_id → parent。
    child_to_parent: Mutex<HashMap<String, String>>,
    /// task_type → executor。
    executors: Arc<Mutex<HashMap<String, Arc<dyn TaskExecutor>>>>,
    /// 每个 session 当前 working 的任务(session_id → task_id),用于并发冲突检测。
    working: Mutex<HashMap<String, String>>,
    /// 创建序号(同优先级稳定排序)。
    order: Mutex<std::collections::HashMap<String, u64>>,
    next_order: Mutex<u64>,
    snapshot_store: Option<Arc<dyn TaskSnapshotStore>>,
}

impl LocalController {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            parent_to_children: Mutex::new(HashMap::new()),
            child_to_parent: Mutex::new(HashMap::new()),
            executors: Arc::new(Mutex::new(HashMap::new())),
            working: Mutex::new(HashMap::new()),
            order: Mutex::new(HashMap::new()),
            next_order: Mutex::new(0),
            snapshot_store: None,
        }
    }

    pub fn with_snapshot_store(
        mut self,
        store: Arc<dyn TaskSnapshotStore>,
    ) -> Result<Self, ControllerError> {
        let restored = store.load()?;
        let mut seen_ids = HashSet::new();
        let mut working_sessions = HashSet::new();
        let restored_ids: HashSet<_> = restored.iter().map(|task| task.task_id.clone()).collect();
        for task in restored {
            if task.task_id.trim().is_empty()
                || task.session_id.trim().is_empty()
                || task.task_type.trim().is_empty()
            {
                return Err(ControllerError(
                    "invalid task snapshot: required field is empty".to_string(),
                ));
            }
            if task.priority < 0 {
                return Err(ControllerError(
                    "invalid task snapshot: priority must be non-negative".to_string(),
                ));
            }
            if !seen_ids.insert(task.task_id.clone()) {
                return Err(ControllerError(format!(
                    "duplicate task snapshot: {}",
                    task.task_id
                )));
            }
            if task.status == TaskStatus::Working
                && !working_sessions.insert(task.session_id.clone())
            {
                return Err(ControllerError(format!(
                    "multiple working tasks for session: {}",
                    task.session_id
                )));
            }
            self.order
                .lock()
                .unwrap()
                .insert(task.task_id.clone(), self.next_seq());
            if task.status == TaskStatus::Working {
                self.working
                    .lock()
                    .unwrap()
                    .insert(task.session_id.clone(), task.task_id.clone());
            }
            if let Some(parent) = &task.parent_task_id {
                if !restored_ids.contains(parent) {
                    return Err(ControllerError(format!(
                        "parent task missing from snapshot: {parent}"
                    )));
                }
                self.parent_to_children
                    .lock()
                    .unwrap()
                    .entry(parent.clone())
                    .or_default()
                    .insert(task.task_id.clone());
                self.child_to_parent
                    .lock()
                    .unwrap()
                    .insert(task.task_id.clone(), parent.clone());
            }
            self.tasks
                .lock()
                .unwrap()
                .insert(task.task_id.clone(), task);
        }
        self.snapshot_store = Some(store);
        Ok(self)
    }

    fn persist(&self) -> Result<(), ControllerError> {
        if let Some(store) = &self.snapshot_store {
            let tasks: Vec<_> = self.tasks.lock().unwrap().values().cloned().collect();
            store.save(&tasks)?;
        }
        Ok(())
    }

    fn next_seq(&self) -> u64 {
        let mut n = self.next_order.lock().unwrap();
        *n += 1;
        *n
    }

    pub fn set_task_error(
        &self,
        task_id: &str,
        message: impl Into<String>,
    ) -> Result<(), ControllerError> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| ControllerError(format!("task not found: {task_id}")))?;
        task.error_message = Some(message.into());
        drop(tasks);
        self.persist()
    }

    pub fn retry_task(&self, task_id: &str) -> Result<(), ControllerError> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| ControllerError(format!("task not found: {task_id}")))?;
        if task.status != TaskStatus::Failed {
            return Err(ControllerError(format!("task is not failed: {task_id}")));
        }
        task.status = TaskStatus::Submitted;
        task.error_message = None;
        drop(tasks);
        self.persist()
    }
}

impl Default for LocalController {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for LocalController {}

#[async_trait]
impl Controller for LocalController {
    fn create_task(&self, task: Task) -> Result<Task, ControllerError> {
        if task.task_id.trim().is_empty()
            || task.session_id.trim().is_empty()
            || task.task_type.trim().is_empty()
        {
            return Err(ControllerError(
                "task_id/session_id/task_type must not be empty".to_string(),
            ));
        }
        if task.priority < 0 {
            return Err(ControllerError("priority must be non-negative".to_string()));
        }
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.contains_key(&task.task_id) {
            return Err(ControllerError(format!(
                "task already exists: {}",
                task.task_id
            )));
        }
        self.order
            .lock()
            .unwrap()
            .insert(task.task_id.clone(), self.next_seq());
        tasks.insert(task.task_id.clone(), task.clone());
        drop(tasks);
        self.persist()?;
        Ok(task)
    }

    fn get_task(&self, task_id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().get(task_id).cloned()
    }

    fn filter_tasks(&self, filter: &TaskFilter) -> Result<Vec<Task>, ControllerError> {
        if filter.task_id.is_none()
            && filter.session_id.is_none()
            && filter.status.is_none()
            && !filter.is_root
        {
            return Err(ControllerError(
                "at least one filter condition required".to_string(),
            ));
        }
        let tasks = self.tasks.lock().unwrap();
        let children = self.parent_to_children.lock().unwrap();
        let mut found: Vec<Task> = tasks
            .values()
            .filter(|task| {
                if let Some(id) = &filter.task_id
                    && task.task_id != *id
                {
                    return false;
                }
                if let Some(session) = &filter.session_id
                    && task.session_id != *session
                {
                    return false;
                }
                if let Some(status) = filter.status
                    && task.status != status
                {
                    return false;
                }
                if filter.is_root && children.values().any(|set| set.contains(&task.task_id)) {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        found.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        Ok(found)
    }

    fn remove_task(&self, task_id: &str) -> Result<(), ControllerError> {
        let mut tasks = self.tasks.lock().unwrap();
        if !tasks.contains_key(task_id) {
            return Err(ControllerError(format!("task not found: {task_id}")));
        }
        if self
            .parent_to_children
            .lock()
            .unwrap()
            .get(task_id)
            .map(|set| !set.is_empty())
            .unwrap_or(false)
        {
            return Err(ControllerError(format!(
                "task has children: {task_id} (remove children first)"
            )));
        }
        if let Some(parent) = self.child_to_parent.lock().unwrap().remove(task_id) {
            let mut parents = self.parent_to_children.lock().unwrap();
            if let Some(children) = parents.get_mut(&parent) {
                children.remove(task_id);
                if children.is_empty() {
                    parents.remove(&parent);
                }
            }
        }
        let session_id = tasks.get(task_id).map(|task| task.session_id.clone());
        self.order.lock().unwrap().remove(task_id);
        if let Some(session_id) = session_id {
            let mut working = self.working.lock().unwrap();
            if working.get(&session_id).is_some_and(|id| id == task_id) {
                working.remove(&session_id);
            }
        }
        tasks.remove(task_id);
        drop(tasks);
        self.persist()?;
        Ok(())
    }

    fn update_status(&self, task_id: &str, status: TaskStatus) -> Result<(), ControllerError> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| ControllerError(format!("task not found: {task_id}")))?;
        let from = task.status;
        if from == status {
            return Ok(());
        }
        if !allowed_transitions(from).contains(&status) {
            return Err(ControllerError(format!(
                "illegal transition {from:?} -> {status:?} for {task_id}"
            )));
        }
        let session_id = task.session_id.clone();
        let mut working = self.working.lock().unwrap();
        match status {
            TaskStatus::Working => {
                if let Some(existing) = working.get(&session_id)
                    && existing != task_id
                {
                    return Err(ControllerError(format!(
                        "session {session_id} already has working task {existing}"
                    )));
                }
                working.insert(session_id, task_id.to_string());
            }
            TaskStatus::Completed
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Paused => {
                if working.get(&session_id).is_some_and(|id| id == task_id) {
                    working.remove(&session_id);
                }
            }
            _ => {}
        }
        task.status = status;
        drop(working);
        drop(tasks);
        self.persist()?;
        Ok(())
    }

    fn link_parent(&self, task_id: &str, parent_task_id: &str) -> Result<(), ControllerError> {
        if task_id == parent_task_id {
            return Err(ControllerError("task cannot be its own parent".to_string()));
        }
        let (task_exists, parent_exists) = {
            let tasks = self.tasks.lock().unwrap();
            (
                tasks.contains_key(task_id),
                tasks.contains_key(parent_task_id),
            )
        };
        if !task_exists {
            return Err(ControllerError(format!("task not found: {task_id}")));
        }
        if !parent_exists {
            return Err(ControllerError(format!(
                "parent not found: {parent_task_id}"
            )));
        }
        // 防环:沿父链向上,不能回到自身。
        let child_to_parent = self.child_to_parent.lock().unwrap();
        let mut cursor = Some(parent_task_id.to_string());
        while let Some(current) = cursor {
            if current == task_id {
                return Err(ControllerError(
                    "cycle detected in task hierarchy".to_string(),
                ));
            }
            cursor = child_to_parent.get(&current).cloned();
        }
        let previous_parent = child_to_parent.get(task_id).cloned();
        drop(child_to_parent);
        if let Some(previous_parent) = previous_parent
            && previous_parent != parent_task_id
        {
            self.parent_to_children
                .lock()
                .unwrap()
                .get_mut(&previous_parent)
                .map(|children| children.remove(task_id));
        }
        self.child_to_parent
            .lock()
            .unwrap()
            .insert(task_id.to_string(), parent_task_id.to_string());
        self.parent_to_children
            .lock()
            .unwrap()
            .entry(parent_task_id.to_string())
            .or_default()
            .insert(task_id.to_string());
        if let Some(task) = self.tasks.lock().unwrap().get_mut(task_id) {
            task.parent_task_id = Some(parent_task_id.to_string());
        }
        self.persist()?;
        Ok(())
    }

    fn pending_tasks(&self, session_id: &str) -> Vec<Task> {
        let tasks = self.tasks.lock().unwrap();
        let order = self.order.lock().unwrap();
        let mut pending: Vec<Task> = tasks
            .values()
            .filter(|t| t.session_id == session_id && t.status == TaskStatus::Submitted)
            .cloned()
            .collect();
        pending.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then(order.get(&a.task_id).cmp(&order.get(&b.task_id)))
                .then(a.task_id.cmp(&b.task_id))
        });
        pending
    }

    fn register_executor(&self, executor: Arc<dyn TaskExecutor>) -> Effect {
        let task_type = executor.task_type().to_string();
        self.executors
            .lock()
            .unwrap()
            .insert(task_type.clone(), executor);
        let executors = self.executors.clone();
        Effect::new(move || {
            executors.lock().unwrap().remove(&task_type);
        })
    }

    async fn run_task(&self, task_id: &str) -> Result<String, ControllerError> {
        let task = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| ControllerError(format!("task not found: {task_id}")))?;
        if task.status != TaskStatus::Submitted && task.status != TaskStatus::Waiting {
            return Err(ControllerError(format!(
                "task not runnable (status {:?})",
                task.status
            )));
        }
        // 冲突处理:同会话已有 working 任务 → 拒绝(块作用域确保 guard 先释放)。
        {
            let working = self.working.lock().unwrap();
            if let Some(existing) = working.get(&task.session_id)
                && existing != task_id
            {
                return Err(ControllerError(format!(
                    "session {} already has working task {existing}",
                    task.session_id
                )));
            }
        }

        let executor = self
            .executors
            .lock()
            .unwrap()
            .get(&task.task_type)
            .cloned()
            .ok_or_else(|| ControllerError(format!("no executor for type {}", task.task_type)))?;

        self.update_status(task_id, TaskStatus::Working)?;
        match executor.execute(&task).await {
            Ok(output) => {
                self.update_status(task_id, TaskStatus::Completed)?;
                Ok(output)
            }
            Err(error) => {
                let _ = self.update_status(task_id, TaskStatus::Failed);
                if let Some(task) = self.tasks.lock().unwrap().get_mut(task_id) {
                    task.error_message = Some(error.0.clone());
                }
                self.persist()?;
                Err(error)
            }
        }
    }

    async fn cancel_task(&self, task_id: &str) -> Result<(), ControllerError> {
        let task = self
            .tasks
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .ok_or_else(|| ControllerError(format!("task not found: {task_id}")))?;
        match task.status {
            TaskStatus::Working
            | TaskStatus::Submitted
            | TaskStatus::Waiting
            | TaskStatus::Paused => self.update_status(task_id, TaskStatus::Canceled),
            TaskStatus::Completed | TaskStatus::Canceled => {
                Err(ControllerError(format!("task already final: {task_id}")))
            }
            _ => self.update_status(task_id, TaskStatus::Canceled),
        }
    }

    fn retry_task(&self, task_id: &str) -> Result<(), ControllerError> {
        LocalController::retry_task(self, task_id)
    }

    fn recognize_intent(&self, query: &str) -> Intent {
        let lower = query.to_lowercase();
        let has = |keywords: &[&str]| keywords.iter().any(|k| lower.contains(k));
        let (intent_type, task_text, confidence): (IntentType, Option<String>, f64) =
            if has(&["pause", "暂停"]) {
                (IntentType::PauseTask, None, 0.9)
            } else if has(&["resume", "恢复"]) {
                (IntentType::ResumeTask, None, 0.9)
            } else if has(&["cancel", "取消"]) {
                (IntentType::CancelTask, None, 0.9)
            } else if has(&["retry", "重试"]) {
                (IntentType::RetryTask, None, 0.9)
            } else if has(&["continue", "继续"]) {
                (IntentType::ContinueTask, Some(query.to_string()), 0.85)
            } else if has(&["switch", "切换"]) {
                (IntentType::SwitchTask, Some(query.to_string()), 0.85)
            } else if has(&["modify", "修改", "更新"]) {
                (IntentType::ModifyTask, Some(query.to_string()), 0.8)
            } else if has(&["supplement", "补充"]) {
                (IntentType::SupplementTask, Some(query.to_string()), 0.8)
            } else if has(&["create", "新建", "做", "执行", "任务"]) {
                (IntentType::CreateTask, Some(query.to_string()), 0.8)
            } else {
                (IntentType::UnknownTask, None, 0.5)
            };
        Intent {
            intent_type,
            task_text,
            task_id: extract_task_id(query),
            confidence,
        }
    }

    async fn recognize_intent_with_llm(
        &self,
        query: &str,
        llm: Arc<dyn ModelProvider>,
    ) -> Result<Intent, ControllerError> {
        let response = llm
            .chat(ModelRequest {
                messages: vec![ChatMessage::new(
                    ChatRole::System,
                    "Classify the request. Reply with only JSON: {\"intent_type\":\"create_task|pause_task|resume_task|retry_task|continue_task|supplement_task|cancel_task|modify_task|switch_task|unknown_task\",\"task_id\":null,\"task_text\":null,\"confidence\":0.0}",
                ), ChatMessage::new(ChatRole::User, query)],
                ..Default::default()
            })
            .await
            .map_err(|error| ControllerError(format!("intent LLM failed: {error}")))?;
        let value = extract_json_object(&response.content)
            .ok_or_else(|| ControllerError("intent LLM returned unparseable JSON".to_string()))?;
        let intent: Intent = serde_json::from_value(value)
            .map_err(|error| ControllerError(format!("invalid intent response: {error}")))?;
        if !(0.0..=1.0).contains(&intent.confidence) {
            return Err(ControllerError(
                "intent confidence must be between 0 and 1".to_string(),
            ));
        }
        Ok(intent)
    }
}

fn extract_task_id(query: &str) -> Option<String> {
    query.split_whitespace().find_map(|token| {
        let id =
            token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
        (id.starts_with("task-") && id.len() > 5).then(|| id.to_string())
    })
}

fn extract_json_object(content: &str) -> Option<serde_json::Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    (end > start).then(|| serde_json::from_str(&content[start..=end]).ok())?
}

/// 控制器插件:提供 controller seam。
pub struct ControllerPlugin {
    pub snapshot_path: Option<PathBuf>,
}

impl ControllerPlugin {
    pub fn new() -> Self {
        Self {
            snapshot_path: None,
        }
    }

    pub fn with_snapshot_path(path: impl Into<PathBuf>) -> Self {
        Self {
            snapshot_path: Some(path.into()),
        }
    }
}

impl Default for ControllerPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for ControllerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-controller"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CONTROLLER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let controller = if let Some(path) = &self.snapshot_path {
            let store: Arc<dyn TaskSnapshotStore> = Arc::new(JsonTaskSnapshotStore::new(path));
            LocalController::new().with_snapshot_store(store)
        } else if let Some(store) = ctx.service::<dyn TaskSnapshotStore>(&TASK_SNAPSHOT_STORE) {
            LocalController::new().with_snapshot_store(store)
        } else {
            Ok(LocalController::new())
        }
        .map_err(|error| PluginError::Apply {
            plugin: self.name(),
            message: error.0,
        })?;
        let controller: Arc<dyn Controller> = Arc::new(controller);
        Ok(vec![ctx.register(CONTROLLER, controller)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::CONTROLLER;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    #[test]
    fn json_snapshot_store_persists_and_rejects_corruption() {
        let path =
            std::env::temp_dir().join(format!("ah-task-snapshot-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = JsonTaskSnapshotStore::new(&path);
        let task = Task::submitted("session", "task", "agent", "run", 1);
        store.save(std::slice::from_ref(&task)).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"version\": 1"));
        assert_eq!(store.load().unwrap(), vec![task]);
        std::fs::write(&path, b"not json").unwrap();
        assert!(store.load().unwrap_err().0.contains("snapshot decode"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_malformed_snapshot_envelope() {
        let path = std::env::temp_dir().join(format!(
            "ah-task-snapshot-malformed-envelope-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, br#"{"version":1}"#).unwrap();
        let error = JsonTaskSnapshotStore::new(&path).load().unwrap_err();
        assert!(error.0.contains("snapshot envelope"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn loads_legacy_task_array_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "ah-task-snapshot-legacy-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let task = Task::submitted("session", "legacy", "agent", "run", 1);
        std::fs::write(
            &path,
            serde_json::to_vec(std::slice::from_ref(&task)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            JsonTaskSnapshotStore::new(&path).load().unwrap(),
            vec![task]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_unsupported_snapshot_version() {
        let path = std::env::temp_dir().join(format!(
            "ah-task-snapshot-version-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, br#"{"version":99,"tasks":[]}"#).unwrap();
        let error = JsonTaskSnapshotStore::new(&path).load().unwrap_err();
        assert!(error.0.contains("snapshot version unsupported"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn plugin_uses_explicit_snapshot_path() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-plugin-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = JsonTaskSnapshotStore::new(&path);
        let task = Task::submitted("session", "task", "agent", "run", 1);
        store.save(std::slice::from_ref(&task)).unwrap();
        let ctx = Context::new();
        let effects = ControllerPlugin::with_snapshot_path(&path)
            .apply(&ctx)
            .unwrap();
        let controller = ctx.service::<dyn Controller>(&CONTROLLER).unwrap();
        assert_eq!(controller.get_task("task"), Some(task));
        drop(effects);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn json_snapshot_store_serializes_concurrent_writes() {
        let path = std::env::temp_dir().join(format!(
            "ah-controller-concurrent-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let mut handles = Vec::new();
        for index in 0..8 {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                let task =
                    Task::submitted("session", format!("task-{index}"), "agent", "run", index);
                store.save(&[task]).unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(store.load().unwrap().len(), 1);
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_snapshot_replace_cleans_lock() {
        let root =
            std::env::temp_dir().join(format!("ah-controller-failed-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let store = JsonTaskSnapshotStore::new(&root);
        let error = store.save(&[]).unwrap_err();
        assert!(error.0.contains("snapshot replace"));
        assert!(!root.with_extension("lock").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_snapshot_lock_conflict() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-lock-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("lock"));
        let first = JsonTaskSnapshotStore::new(&path);
        let second = JsonTaskSnapshotStore::new(&path);
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let lock_path = path.with_extension("lock");
        let _lock = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .unwrap();
        let result = second.save(&[]);
        assert!(matches!(result, Err(error) if error.0.contains("snapshot lock")));
        drop(_lock);
        let _ = std::fs::remove_file(lock_path);
        let _ = std::fs::remove_file(path);
        let _ = first;
    }

    #[test]
    fn rejects_invalid_task_snapshot_fields() {
        let store = Arc::new(InMemoryTaskSnapshotStore::new());
        store
            .save(&[Task::submitted("", "task", "agent", "run", 1)])
            .unwrap();
        let result = LocalController::new().with_snapshot_store(store);
        assert!(matches!(result, Err(error) if error.0.contains("required field is empty")));
        let store = Arc::new(InMemoryTaskSnapshotStore::new());
        store
            .save(&[Task::submitted("session", "task", "agent", "run", -1)])
            .unwrap();
        let result = LocalController::new().with_snapshot_store(store);
        assert!(matches!(result, Err(error) if error.0.contains("priority must be non-negative")));
    }

    #[test]
    fn rejects_inconsistent_task_snapshot() {
        let store = Arc::new(InMemoryTaskSnapshotStore::new());
        let mut child = Task::submitted("session", "child", "agent", "run", 1);
        child.parent_task_id = Some("missing".into());
        store.save(&[child]).unwrap();
        let result = LocalController::new().with_snapshot_store(store);
        assert!(matches!(result, Err(error) if error.0.contains("parent task missing")));
    }

    #[test]
    fn plugin_rejects_corrupt_explicit_snapshot() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-corrupt-{}.json", std::process::id()));
        std::fs::write(&path, b"not json").unwrap();
        let ctx = Context::new();
        let error = ControllerPlugin::with_snapshot_path(&path)
            .apply(&ctx)
            .unwrap_err();
        assert!(
            matches!(error, PluginError::Apply { message, .. } if message.contains("snapshot decode"))
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn controller_restores_and_persists_tasks() {
        let path = std::env::temp_dir().join(format!("ah-controller-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let controller = LocalController::new()
            .with_snapshot_store(store.clone())
            .unwrap();
        controller
            .create_task(Task::submitted("session", "parent", "agent", "run", 1))
            .unwrap();
        controller
            .create_task(Task::submitted("session", "task", "agent", "run", 1))
            .unwrap();
        controller.link_parent("task", "parent").unwrap();
        assert!(
            LocalController::new()
                .with_snapshot_store(store)
                .unwrap()
                .get_task("task")
                .is_some()
        );
        let restored = LocalController::new()
            .with_snapshot_store(Arc::new(JsonTaskSnapshotStore::new(&path)))
            .unwrap();
        assert_eq!(
            restored.get_task("task").unwrap().parent_task_id.as_deref(),
            Some("parent")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_task_error_is_restored_from_snapshot() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-error-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let mut task = Task::submitted("session", "task", "agent", "run", 1);
        task.status = TaskStatus::Failed;
        task.error_message = Some("executor failed".into());
        store.save(&[task]).unwrap();
        let restored = LocalController::new().with_snapshot_store(store).unwrap();
        assert_eq!(
            restored.get_task("task").unwrap().error_message.as_deref(),
            Some("executor failed")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn removing_child_cleans_parent_index_and_persists() {
        let path = std::env::temp_dir().join(format!(
            "ah-controller-remove-child-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let controller = LocalController::new()
            .with_snapshot_store(store.clone())
            .unwrap();
        controller
            .create_task(Task::submitted("session", "parent", "agent", "run", 1))
            .unwrap();
        controller
            .create_task(Task::submitted("session", "child", "agent", "run", 1))
            .unwrap();
        controller.link_parent("child", "parent").unwrap();
        controller.remove_task("child").unwrap();
        assert!(controller.remove_task("parent").is_ok());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn relinking_parent_removes_old_index_and_persists_new_one() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-relink-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let controller = LocalController::new()
            .with_snapshot_store(store.clone())
            .unwrap();
        controller
            .create_task(Task::submitted("session", "parent-a", "agent", "run", 1))
            .unwrap();
        controller
            .create_task(Task::submitted("session", "parent-b", "agent", "run", 1))
            .unwrap();
        controller
            .create_task(Task::submitted("session", "child", "agent", "run", 1))
            .unwrap();
        controller.link_parent("child", "parent-a").unwrap();
        controller.link_parent("child", "parent-b").unwrap();
        assert!(controller.remove_task("parent-a").is_ok());
        let restored = LocalController::new().with_snapshot_store(store).unwrap();
        assert_eq!(
            restored
                .get_task("child")
                .unwrap()
                .parent_task_id
                .as_deref(),
            Some("parent-b")
        );
        assert!(
            restored
                .remove_task("parent-b")
                .unwrap_err()
                .0
                .contains("has children")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn restores_parent_child_indexes() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-parent-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let parent = Task::submitted("session", "parent", "agent", "run", 1);
        let mut child = Task::submitted("session", "child", "agent", "run", 1);
        child.parent_task_id = Some("parent".into());
        store.save(&[parent, child]).unwrap();
        let controller = LocalController::new().with_snapshot_store(store).unwrap();
        assert!(
            controller
                .remove_task("parent")
                .unwrap_err()
                .0
                .contains("has children")
        );
        let roots = controller
            .filter_tasks(&TaskFilter {
                is_root: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].task_id, "parent");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn restores_working_index_and_rejects_session_conflict() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-working-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Arc::new(JsonTaskSnapshotStore::new(&path));
        let mut working = Task::submitted("session", "working", "agent", "echo", 1);
        working.status = TaskStatus::Working;
        store.save(&[working]).unwrap();
        let controller = LocalController::new().with_snapshot_store(store).unwrap();
        controller
            .create_task(Task::submitted("session", "pending", "agent", "echo", 1))
            .unwrap();
        let _executor_effect = controller.register_executor(Arc::new(EchoExecutor));
        let error = controller.run_task("pending").await.unwrap_err();
        assert!(error.0.contains("already has working task working"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn snapshot_store_round_trips_tasks() {
        let store = InMemoryTaskSnapshotStore::new();
        let task = Task::submitted("session", "task", "agent", "run", 1);
        store.save(std::slice::from_ref(&task)).unwrap();
        assert_eq!(store.load().unwrap(), vec![task]);
        store.save(&[]).unwrap();
        assert!(store.load().unwrap().is_empty());
    }

    fn build_ctx() -> (Context, Vec<Effect>) {
        let root = std::env::temp_dir().join(format!(
            "ah-ctrl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(ControllerPlugin::default()),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let _ = std::fs::remove_dir_all(&root);
        (ctx, effects)
    }

    /// 回显执行器:task.payload 里的 "echo" 直接返回。
    struct EchoExecutor;

    #[async_trait]
    impl TaskExecutor for EchoExecutor {
        fn task_type(&self) -> &'static str {
            "echo"
        }

        async fn execute(&self, task: &Task) -> Result<String, ControllerError> {
            let echo = task
                .payload
                .get("echo")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(no echo)");
            Ok(format!("echoed:{echo}"))
        }

        fn can_pause(&self, _task: &Task) -> bool {
            true
        }
    }

    #[test]
    fn retry_resets_failed_task_and_persists() {
        let path =
            std::env::temp_dir().join(format!("ah-controller-retry-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let controller = LocalController::new()
            .with_snapshot_store(Arc::new(JsonTaskSnapshotStore::new(&path)))
            .unwrap();
        let task = Task::submitted("session", "retry-task", "agent", "run", 1);
        controller.create_task(task).unwrap();
        controller
            .update_status("retry-task", TaskStatus::Working)
            .unwrap();
        controller
            .update_status("retry-task", TaskStatus::Failed)
            .unwrap();
        controller
            .set_task_error("retry-task", "temporary failure")
            .unwrap();
        controller.retry_task("retry-task").unwrap();
        let retried = controller.get_task("retry-task").unwrap();
        assert_eq!(retried.status, TaskStatus::Submitted);
        assert_eq!(retried.error_message, None);
        assert_eq!(
            JsonTaskSnapshotStore::new(&path).load().unwrap()[0].status,
            TaskStatus::Submitted
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retry_rejects_non_failed_and_unknown_tasks() {
        let controller = LocalController::default();
        controller
            .create_task(Task::submitted("session", "task", "agent", "run", 1))
            .unwrap();
        assert!(
            controller
                .retry_task("task")
                .unwrap_err()
                .0
                .contains("failed")
        );
        assert!(
            controller
                .retry_task("missing")
                .unwrap_err()
                .0
                .contains("not found")
        );
    }

    #[test]
    fn task_lifecycle_and_validation() {
        let (ctx, effects) = build_ctx();
        let controller = ctx
            .service::<dyn Controller>(&CONTROLLER)
            .expect("controller");

        let task = Task::submitted("s1", "t1", "echo", "say hi", 1);
        let created = controller.create_task(task.clone()).expect("create");
        assert_eq!(created.status, TaskStatus::Submitted);
        assert!(
            controller
                .create_task(Task::submitted("s1", "t1", "echo", "dup", 1))
                .is_err(),
            "duplicate id rejected"
        );
        assert!(
            controller
                .create_task(Task::submitted("", "t2", "echo", "empty session", 1))
                .is_err(),
            "empty session rejected"
        );
        assert!(
            controller
                .create_task(Task::submitted("s1", "t3", "echo", "neg priority", -1))
                .is_err(),
            "negative priority rejected"
        );

        // 非法迁移拒绝。
        controller
            .update_status("t1", TaskStatus::Working)
            .expect("submitted->working");
        assert!(
            controller
                .update_status("t1", TaskStatus::Submitted)
                .is_err(),
            "working->submitted illegal"
        );
        controller
            .update_status("t1", TaskStatus::Paused)
            .expect("working->paused");
        controller
            .update_status("t1", TaskStatus::Submitted)
            .expect("paused->submitted");
        controller
            .update_status("t1", TaskStatus::Working)
            .expect("submitted->working");
        controller
            .update_status("t1", TaskStatus::Failed)
            .expect("working->failed");
        assert!(
            controller
                .update_status("t1", TaskStatus::Submitted)
                .is_err(),
            "failed is terminal"
        );

        // 过滤器:至少一个条件。
        assert!(controller.filter_tasks(&TaskFilter::default()).is_err());

        drop(effects);
    }

    #[test]
    fn hierarchy_and_priority() {
        let (ctx, effects) = build_ctx();
        let controller = ctx
            .service::<dyn Controller>(&CONTROLLER)
            .expect("controller");

        controller
            .create_task(Task::submitted("s1", "root", "echo", "root", 2))
            .expect("root");
        controller
            .create_task(Task::submitted("s1", "child", "echo", "child", 1))
            .expect("child");
        controller
            .link_parent("child", "root")
            .expect("link child->root");
        assert!(
            controller.link_parent("root", "child").is_err(),
            "cycle rejected"
        );
        assert!(
            controller.link_parent("child", "missing").is_err(),
            "missing parent rejected"
        );

        // 有子任务不可删除。
        assert!(controller.remove_task("root").is_err());

        // 优先级:root(2) 在 child(1) 之后。
        let pending = controller.pending_tasks("s1");
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].task_id, "child", "priority 1 first");
        assert_eq!(pending[1].task_id, "root");

        // 过滤器:root。
        let roots = controller
            .filter_tasks(&TaskFilter {
                is_root: true,
                ..Default::default()
            })
            .expect("root filter");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].task_id, "root");

        drop(effects);
    }

    #[tokio::test]
    async fn scheduler_runs_real_executor_and_conflict_blocks() {
        let (ctx, effects) = build_ctx();
        let controller = ctx
            .service::<dyn Controller>(&CONTROLLER)
            .expect("controller");
        let _exec = controller.register_executor(StdArc::new(EchoExecutor));

        let mut task = Task::submitted("s1", "a", "echo", "first", 1);
        task.payload = serde_json::json!({ "echo": "hello" });
        controller.create_task(task).expect("create a");
        let mut task_b = Task::submitted("s1", "b", "echo", "second", 1);
        task_b.payload = serde_json::json!({ "echo": "world" });
        controller.create_task(task_b).expect("create b");

        let output = controller.run_task("a").await.expect("run a");
        assert_eq!(output, "echoed:hello");
        assert_eq!(
            controller.get_task("a").expect("a").status,
            TaskStatus::Completed
        );

        // 同会话冲突:a 已 completed → 可跑 b。
        let output_b = controller.run_task("b").await.expect("run b");
        assert_eq!(output_b, "echoed:world");

        // 无执行器类型显式报错。
        controller
            .create_task(Task::submitted("s1", "c", "nope", "x", 1))
            .expect("create c");
        assert!(controller.run_task("c").await.is_err());

        drop(effects);
    }

    #[test]
    fn intent_recognition_is_deterministic() {
        let (ctx, effects) = build_ctx();
        let controller = ctx
            .service::<dyn Controller>(&CONTROLLER)
            .expect("controller");

        assert_eq!(
            controller
                .recognize_intent("please create a report")
                .intent_type,
            IntentType::CreateTask
        );
        assert_eq!(
            controller.recognize_intent("pause that").intent_type,
            IntentType::PauseTask
        );
        assert_eq!(
            controller.recognize_intent("resume it").intent_type,
            IntentType::ResumeTask
        );
        assert_eq!(
            controller.recognize_intent("cancel everything").intent_type,
            IntentType::CancelTask
        );
        assert_eq!(
            controller.recognize_intent("continue the work").intent_type,
            IntentType::ContinueTask
        );
        assert_eq!(
            controller
                .recognize_intent("switch to another task")
                .intent_type,
            IntentType::SwitchTask
        );
        assert_eq!(
            controller.recognize_intent("hello there").intent_type,
            IntentType::UnknownTask
        );

        drop(effects);
    }
}
