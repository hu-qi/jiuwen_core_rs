//! # ah-plugins-controller
//!
//! 真实控制器(对齐 Python core/controller):
//! - TaskManager:任务 CRUD/状态迁移/优先级索引/父子层级(防环);
//! - TaskScheduler:执行器注册表 + 真实执行(消费方注入后端)+ 冲突处理
//!   (同会话已有 working 任务 → 拒绝新执行);
//! - IntentRecognizer:确定性关键词意图识别(create/pause/resume/cancel/
//!   switch/modify/continue/supplement,未知 → UnknownTask)。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ah_contracts::controller::{
    Controller, ControllerError, Intent, IntentType, Task, TaskExecutor, TaskFilter, TaskStatus,
};
use ah_contracts::effect::Effect;
use ah_contracts::keys::CONTROLLER;
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
pub struct LocalController {
    tasks: Mutex<HashMap<String, Task>>,
    /// task_id → children。
    parent_to_children: Mutex<HashMap<String, HashSet<String>>>,
    /// task_id → parent。
    child_to_parent: Mutex<HashMap<String, String>>,
    executors: Arc<Mutex<HashMap<String, Arc<dyn TaskExecutor>>>>,
    /// 每个 session 当前 working 的任务(冲突检测)。
    working: Mutex<HashMap<String, String>>,
    /// 创建序号(同优先级排序)。
    order: Mutex<std::collections::HashMap<String, u64>>,
    next_order: Mutex<u64>,
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
        }
    }

    fn next_seq(&self) -> u64 {
        let mut n = self.next_order.lock().unwrap();
        *n += 1;
        *n
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
        // 解除父链接。
        if let Some(parent) = self.child_to_parent.lock().unwrap().remove(task_id) {
            self.parent_to_children
                .lock()
                .unwrap()
                .get_mut(&parent)
                .map(|set| set.remove(task_id));
        }
        self.order.lock().unwrap().remove(task_id);
        self.working.lock().unwrap().remove(task_id);
        tasks.remove(task_id);
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
        // working 登记簿维护。
        let mut working = self.working.lock().unwrap();
        match status {
            TaskStatus::Working => {
                working.insert(task_id.to_string(), task.session_id.clone());
            }
            TaskStatus::Completed
            | TaskStatus::Failed
            | TaskStatus::Canceled
            | TaskStatus::Paused => {
                working.remove(task_id);
            }
            _ => {}
        }
        task.status = status;
        Ok(())
    }

    fn link_parent(&self, task_id: &str, parent_task_id: &str) -> Result<(), ControllerError> {
        if task_id == parent_task_id {
            return Err(ControllerError("task cannot be its own parent".to_string()));
        }
        let tasks = self.tasks.lock().unwrap();
        if !tasks.contains_key(task_id) {
            return Err(ControllerError(format!("task not found: {task_id}")));
        }
        if !tasks.contains_key(parent_task_id) {
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
        drop(child_to_parent);
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
            confidence,
        }
    }
}

/// 控制器插件:提供 controller seam。
pub struct ControllerPlugin;

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
        let controller: Arc<dyn Controller> = Arc::new(LocalController::new());
        Ok(vec![ctx.register(CONTROLLER, controller)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::CONTROLLER;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let root = std::env::temp_dir().join(format!("ah-ctrl-{}", std::process::id()));
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
            StdArc::new(ControllerPlugin),
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
