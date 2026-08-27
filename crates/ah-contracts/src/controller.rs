//! controller seam:任务管理/调度/意图识别(对齐 Python core/controller)。
//!
//! 状态流:submitted → working → (completed | failed | paused | canceled)
//!                    ↘ input-required → (continue | cancel)

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 任务状态(对齐 TaskStatus)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Submitted,
    Working,
    Paused,
    InputRequired,
    Completed,
    Canceled,
    Failed,
    Waiting,
    Unknown,
}

/// 任务(对齐 Task 模型核心字段)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Task {
    pub session_id: String,
    pub task_id: String,
    pub task_type: String,
    pub description: String,
    /// 越小越优先,默认 1。
    pub priority: i32,
    pub status: TaskStatus,
    pub parent_task_id: Option<String>,
    pub error_message: Option<String>,
    /// 可选执行参数(交给 executor)。
    pub payload: Value,
}

impl Task {
    /// 新建 submitted 任务。
    pub fn submitted(
        session_id: impl Into<String>,
        task_id: impl Into<String>,
        task_type: impl Into<String>,
        description: impl Into<String>,
        priority: i32,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            task_id: task_id.into(),
            task_type: task_type.into(),
            description: description.into(),
            priority,
            status: TaskStatus::Submitted,
            parent_task_id: None,
            error_message: None,
            payload: Value::Null,
        }
    }
}

/// 任务过滤器(对齐 TaskFilter;至少一个条件)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskFilter {
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<TaskStatus>,
    pub is_root: bool,
}

/// 控制器错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerError(pub String);

impl core::fmt::Display for ControllerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ControllerError {}

/// 意图类型(对齐 IntentType)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentType {
    CreateTask,
    PauseTask,
    ResumeTask,
    RetryTask,
    ContinueTask,
    SupplementTask,
    CancelTask,
    ModifyTask,
    SwitchTask,
    UnknownTask,
}

/// 识别出的意图。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Intent {
    pub intent_type: IntentType,
    /// 提取的任务描述(create/continue 等携带)。
    pub task_text: Option<String>,
    pub confidence: f64,
}

/// 任务执行器(抽象):真实执行一个任务,产出文本结果。
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// 支持的 task_type。
    fn task_type(&self) -> &'static str;

    /// 执行任务(消费方注入真实执行后端)。
    async fn execute(&self, task: &Task) -> Result<String, ControllerError>;

    /// 是否可暂停(默认 false;子进程类任务不可暂停)。
    fn can_pause(&self, _task: &Task) -> bool {
        false
    }
}

/// 可持久化任务快照存储 seam。
#[async_trait]
pub trait TaskSnapshotStore: Seam {
    fn save(&self, tasks: &[Task]) -> Result<(), ControllerError>;
    fn load(&self) -> Result<Vec<Task>, ControllerError>;
}

/// 控制器 Seam(Service Definition):任务生命周期 + 调度 + 意图识别。
#[async_trait]
pub trait Controller: Seam {
    // ---- 任务管理(CRUD + 状态 + 优先级 + 层级)----

    /// 创建任务(校验必填字段;task_id 唯一)。
    fn create_task(&self, task: Task) -> Result<Task, ControllerError>;

    /// 按 id 取任务。
    fn get_task(&self, task_id: &str) -> Option<Task>;

    /// 按过滤器查询(至少一个条件;root 任务含无父任务)。
    fn filter_tasks(&self, filter: &TaskFilter) -> Result<Vec<Task>, ControllerError>;

    /// 删除任务(带子任务则拒绝)。
    fn remove_task(&self, task_id: &str) -> Result<(), ControllerError>;

    /// 更新任务状态(非法迁移显式报错)。
    fn update_status(&self, task_id: &str, status: TaskStatus) -> Result<(), ControllerError>;

    /// 建立父子层级(父必须存在;防环)。
    fn link_parent(&self, task_id: &str, parent_task_id: &str) -> Result<(), ControllerError>;

    /// 按优先级取待执行任务(升序,同优先级按创建序)。
    fn pending_tasks(&self, session_id: &str) -> Vec<Task>;

    // ---- 调度 ----

    /// 注册执行器(可逆)。
    fn register_executor(
        &self,
        executor: std::sync::Arc<dyn TaskExecutor>,
    ) -> crate::effect::Effect;

    /// 执行一个已提交任务(经注册表找 executor;冲突处理:同会话已有 working 任务 → 拒绝)。
    async fn run_task(&self, task_id: &str) -> Result<String, ControllerError>;

    /// 取消任务(working 可取消;completed 不可)。
    async fn cancel_task(&self, task_id: &str) -> Result<(), ControllerError>;

    /// 将失败任务重置为 submitted，并清除上次错误。
    fn retry_task(&self, task_id: &str) -> Result<(), ControllerError>;

    // ---- 意图识别 ----

    /// 从用户文本识别意图(确定性关键词;LLM 识别留待后续)。
    fn recognize_intent(&self, query: &str) -> Intent;
}
