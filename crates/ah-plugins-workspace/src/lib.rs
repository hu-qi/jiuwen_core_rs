//! # ah-plugins-workspace
//!
//! 真实工作区清单(对应 openjiuwen/harness 的 workspace/goal/manifest):
//! workspace.json 描述工作区名称与目标状态机;create_goal / update_goal_status
//! 每次变更真实落盘,重启后完整恢复。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::WORKSPACE;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::workspace::{
    DirectoryNode, Goal, GoalStatus, WorkspaceError, WorkspaceManifest, WorkspaceService,
    is_safe_relative_path, validate_directory_node,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实文件后端工作区服务。
pub struct FileWorkspaceService {
    dir: PathBuf,
    manifest: Mutex<Option<WorkspaceManifest>>,
}

impl FileWorkspaceService {
    /// 以工作区目录打开(workspace.json 位于目录根)。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| WorkspaceError(format!("create workspace dir failed: {e}")))?;
        Ok(Self {
            dir,
            manifest: Mutex::new(None),
        })
    }

    fn path(&self) -> PathBuf {
        self.dir.join("workspace.json")
    }

    fn persist(&self, manifest: &WorkspaceManifest) -> Result<(), WorkspaceError> {
        let text = serde_json::to_string_pretty(manifest)
            .map_err(|e| WorkspaceError(format!("serialize manifest: {e}")))?;
        std::fs::write(self.path(), text)
            .map_err(|e| WorkspaceError(format!("write manifest: {e}")))?;
        Ok(())
    }
}

impl Seam for FileWorkspaceService {}

impl WorkspaceService for FileWorkspaceService {
    fn load(&self) -> Result<WorkspaceManifest, WorkspaceError> {
        let mut slot = self.manifest.lock().unwrap();
        if let Some(manifest) = slot.as_ref() {
            return Ok(manifest.clone());
        }
        let path = self.path();
        let manifest = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| WorkspaceError(format!("read manifest: {e}")))?;
            serde_json::from_str(&text)
                .map_err(|e| WorkspaceError(format!("parse manifest: {e}")))?
        } else {
            let now = now_ms();
            let default = WorkspaceManifest {
                name: "default-workspace".to_string(),
                description: String::new(),
                goals: Vec::new(),
                updated_ms: now,
            };
            self.persist(&default)?;
            default
        };
        *slot = Some(manifest.clone());
        Ok(manifest)
    }

    fn save(&self, manifest: &WorkspaceManifest) -> Result<(), WorkspaceError> {
        let mut updated = manifest.clone();
        updated.updated_ms = now_ms();
        self.persist(&updated)?;
        *self.manifest.lock().unwrap() = Some(updated.clone());
        Ok(())
    }

    fn create_goal(&self, id: &str, title: &str) -> Result<Goal, WorkspaceError> {
        if id.is_empty() || title.is_empty() {
            return Err(WorkspaceError(
                "goal id/title must not be empty".to_string(),
            ));
        }
        let mut manifest = self.load()?;
        if manifest.goals.iter().any(|g| g.id == id) {
            return Err(WorkspaceError(format!("goal already exists: {id}")));
        }
        let now = now_ms();
        let goal = Goal {
            id: id.to_string(),
            title: title.to_string(),
            status: GoalStatus::Pending,
            created_ms: now,
            updated_ms: now,
        };
        manifest.goals.push(goal.clone());
        self.save(&manifest)?;
        Ok(goal)
    }

    fn update_goal_status(&self, id: &str, status: GoalStatus) -> Result<Goal, WorkspaceError> {
        let mut manifest = self.load()?;
        let goal = manifest
            .goals
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| WorkspaceError(format!("goal not found: {id}")))?;
        goal.status = status;
        goal.updated_ms = now_ms();
        let updated = goal.clone();
        self.save(&manifest)?;
        Ok(updated)
    }

    fn goals(&self) -> Result<Vec<Goal>, WorkspaceError> {
        let mut goals = self.load()?.goals;
        goals.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(goals)
    }
}

/// 工作区目录构建器(对齐 directory_builder.py `DirectoryBuilder`):
/// 按目录节点树真实创建目录(带 `.workspace` 标记文件)与文件(默认内容)。
/// 不安全路径(绝对路径/盘符/`..` 越级)显式报错,不落盘。
pub struct DirectoryBuilder {
    root_path: PathBuf,
}

impl DirectoryBuilder {
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
        }
    }

    /// 递归创建目录结构(对齐 `build` + `_create_directory_recursive`)。
    pub fn build(&self, node: &DirectoryNode) -> Result<(), WorkspaceError> {
        self.build_recursive(node, "")
    }

    fn build_recursive(
        &self,
        node: &DirectoryNode,
        parent_path: &str,
    ) -> Result<(), WorkspaceError> {
        let relative = &node.path;
        if !is_safe_relative_path(relative) {
            return Err(WorkspaceError(format!("Unsafe path detected: {relative}")));
        }
        let full_path = if parent_path.is_empty() {
            if self.root_path.as_os_str().is_empty() {
                PathBuf::from(relative)
            } else {
                self.root_path.join(relative)
            }
        } else {
            PathBuf::from(parent_path).join(relative)
        };
        let full_path_str = full_path.to_string_lossy().to_string();

        if node.is_file {
            if !full_path.exists() {
                std::fs::create_dir_all(full_path.parent().unwrap_or(&full_path))
                    .map_err(|e| WorkspaceError(format!("create parent dir failed: {e}")))?;
                std::fs::write(&full_path, "")
                    .map_err(|e| WorkspaceError(format!("write file failed: {e}")))?;
            }
        } else {
            std::fs::create_dir_all(&full_path)
                .map_err(|e| WorkspaceError(format!("create dir failed: {e}")))?;
            // 目录标记文件 `.workspace`(空内容,对齐 marker_file)。
            let marker = full_path.join(".workspace");
            if !marker.exists() {
                std::fs::write(&marker, "")
                    .map_err(|e| WorkspaceError(format!("write marker failed: {e}")))?;
            }
        }
        for child in &node.children {
            self.build_recursive(child, &full_path_str)?;
        }
        Ok(())
    }

    /// 按 JSON 节点树构建(供配置驱动;先整体校验再创建)。
    pub fn build_from_json(&self, nodes: &Value) -> Result<(), WorkspaceError> {
        let Some(array) = nodes.as_array() else {
            return Err(WorkspaceError(
                "`directories` must be a list of directory definitions.".to_string(),
            ));
        };
        for node in array {
            validate_directory_node(node)?;
            let parsed: DirectoryNode = serde_json::from_value(node.clone())
                .map_err(|e| WorkspaceError(format!("invalid directory node: {e}")))?;
            self.build(&parsed)?;
        }
        Ok(())
    }
}

/// workspace 插件:提供真实工作区清单。
pub struct WorkspacePlugin {
    dir: PathBuf,
}

impl WorkspacePlugin {
    /// 以工作区目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for WorkspacePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-workspace"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![WORKSPACE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service =
            FileWorkspaceService::open(self.dir.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let service: std::sync::Arc<dyn WorkspaceService> = std::sync::Arc::new(service);
        Ok(vec![ctx.register(WORKSPACE, service)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::WORKSPACE;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(WorkspacePlugin::new(root.join("ws")))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn goal_lifecycle_persists_across_reopen() {
        let root = std::env::temp_dir().join(format!("ah-ws-life-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let ws = ctx
            .service::<dyn WorkspaceService>(&WORKSPACE)
            .expect("workspace");

        // 首次 load 创建默认清单。
        let manifest = ws.load().expect("load");
        assert!(manifest.goals.is_empty());

        let goal = ws.create_goal("g1", "ship harness").expect("create");
        assert_eq!(goal.status, GoalStatus::Pending);
        let updated = ws
            .update_goal_status("g1", GoalStatus::InProgress)
            .expect("update");
        assert_eq!(updated.status, GoalStatus::InProgress);
        ws.update_goal_status("g1", GoalStatus::Done).expect("done");

        // 重复 id 显式报错。
        assert!(ws.create_goal("g1", "dup").is_err());
        // 不存在的目标报错。
        assert!(ws.update_goal_status("missing", GoalStatus::Done).is_err());

        drop(effects);

        // 重启:workspace.json 恢复全部状态。
        let reopened = FileWorkspaceService::open(root.join("ws")).expect("reopen");
        let goals = reopened.goals().expect("goals");
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].status, GoalStatus::Done);
        assert_eq!(goals[0].title, "ship harness");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn manifest_file_is_real_json() {
        let root = std::env::temp_dir().join(format!("ah-ws-file-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let ws = ctx
            .service::<dyn WorkspaceService>(&WORKSPACE)
            .expect("workspace");
        ws.create_goal("g2", "write docs").expect("create");

        let path = root.join("ws").join("workspace.json");
        assert!(path.exists(), "workspace.json written");
        let text = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(parsed["goals"][0]["id"], "g2");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_builder_creates_real_tree() {
        use ah_contracts::workspace::DirectoryNode;
        let root = std::env::temp_dir().join(format!("ah-ws-build-{}", std::process::id()));
        let builder = DirectoryBuilder::new(&root);

        let tree = DirectoryNode::new("memory", "memory", "记忆", false).with_children(vec![
            DirectoryNode::new("MEMORY.md", "MEMORY.md", "索引", true),
            DirectoryNode::new("daily_memory", "daily_memory", "每日", false),
        ]);
        builder.build(&tree).expect("build");

        // 目录 + 标记文件 + 子文件真实落盘。
        assert!(root.join("memory").is_dir());
        assert!(root.join("memory/.workspace").exists(), "marker written");
        assert!(root.join("memory/MEMORY.md").is_file());
        assert!(root.join("memory/daily_memory").is_dir());
        assert!(root.join("memory/daily_memory/.workspace").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_builder_rejects_unsafe_path() {
        use ah_contracts::workspace::DirectoryNode;
        let root = std::env::temp_dir().join(format!("ah-ws-unsafe-{}", std::process::id()));
        let builder = DirectoryBuilder::new(&root);

        // 绝对路径 / 越级路径显式报错,不落盘。
        let abs = DirectoryNode::new("escape", "/etc", "x", false);
        assert!(builder.build(&abs).is_err());
        let traverse = DirectoryNode::new("escape", "../up", "x", false);
        assert!(builder.build(&traverse).is_err());
        assert!(!root.exists() || root.read_dir().unwrap().next().is_none());

        // 合法树不受影响。
        builder
            .build(&DirectoryNode::new("ok", "ok_dir", "x", false))
            .expect("ok");
        assert!(root.join("ok_dir").is_dir());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_builder_from_json_validates_then_builds() {
        let root = std::env::temp_dir().join(format!("ah-ws-json-{}", std::process::id()));
        let builder = DirectoryBuilder::new(&root);

        // 非法节点(带分隔符 name)→ 显式报错,不创建任何内容。
        let bad = serde_json::json!([{"name": "a/b", "path": "x"}]);
        assert!(builder.build_from_json(&bad).is_err());

        // 合法 JSON 数组真实构建。
        let good = serde_json::json!([
            {"name": "skills", "path": "skills", "is_file": false, "children": [
                {"name": "code", "path": "code", "is_file": true}
            ]}
        ]);
        builder.build_from_json(&good).expect("build");
        assert!(root.join("skills").is_dir());
        assert!(root.join("skills/code").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }
}
