//! # ah-plugins-context-evolver
//!
//! 真实任务记忆服务(对齐 Python extensions/context_evolver):
//! - save:任务记忆 JSON 落盘(重启恢复);
//! - retrieve:关键词 + 标签打分,按相关性降序;
//! - summarize:轨迹凝练摘要(确定性归纳,无 LLM);
//! - inject:检索相关记忆 + 凝练摘要 → 注入文本(可并入 system prompt)。

use std::path::PathBuf;
use std::sync::Mutex;

use ah_contracts::context_evolver::{
    MemoryEvolver, MemoryEvolverError, MemoryInjection, TaskMemory, TrajectorySummary,
};
use ah_contracts::evolving::{StepOutcome, Trajectory};
use ah_contracts::keys::MEMORY_EVOLVER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实任务记忆服务。
pub struct JsonTaskMemoryService {
    dir: PathBuf,
    memories: Mutex<Vec<TaskMemory>>,
}

impl JsonTaskMemoryService {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, MemoryEvolverError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| MemoryEvolverError(format!("create dir: {e}")))?;
        let mut memories = Vec::new();
        let path = dir.join("task_memories.jsonl");
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                if let Ok(memory) = serde_json::from_str::<TaskMemory>(line) {
                    memories.push(memory);
                }
            }
        }
        Ok(Self {
            dir,
            memories: Mutex::new(memories),
        })
    }

    /// 打分:任务/内容包含查询词得 2 分,标签命中得 1 分。
    fn score(&self, memory: &TaskMemory, query: &str) -> u64 {
        let lower = query.to_lowercase();
        let mut score = 0;
        if memory.task.to_lowercase().contains(&lower) {
            score += 2;
        }
        if memory.content.to_lowercase().contains(&lower) {
            score += 2;
        }
        for tag in &memory.tags {
            if lower.contains(&tag.to_lowercase()) || tag.to_lowercase().contains(&lower) {
                score += 1;
            }
        }
        score
    }
}

impl Seam for JsonTaskMemoryService {}

impl MemoryEvolver for JsonTaskMemoryService {
    fn save(
        &self,
        task: &str,
        content: &str,
        tags: Vec<String>,
    ) -> Result<TaskMemory, MemoryEvolverError> {
        if task.trim().is_empty() || content.trim().is_empty() {
            return Err(MemoryEvolverError(
                "task and content must not be empty".to_string(),
            ));
        }
        let memory = TaskMemory {
            id: format!("mem-{}-{}", now_ms(), self.memories.lock().unwrap().len()),
            task: task.to_string(),
            content: content.to_string(),
            tags,
            saved_ms: now_ms(),
        };
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("task_memories.jsonl"))
            .map_err(|e| MemoryEvolverError(format!("open memories: {e}")))?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&memory)
                .map_err(|e| MemoryEvolverError(format!("serialize: {e}")))?
        )
        .map_err(|e| MemoryEvolverError(format!("append memory: {e}")))?;
        self.memories.lock().unwrap().push(memory.clone());
        Ok(memory)
    }

    fn retrieve(&self, task: &str, limit: usize) -> Vec<TaskMemory> {
        let mut memories: Vec<(u64, TaskMemory)> = self
            .memories
            .lock()
            .unwrap()
            .iter()
            .map(|m| (self.score(m, task), m.clone()))
            .filter(|(score, _)| *score > 0)
            .collect();
        memories.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        memories.into_iter().map(|(_, m)| m).take(limit).collect()
    }

    fn summarize(&self, trajectories: &[Trajectory]) -> TrajectorySummary {
        if trajectories.is_empty() {
            return TrajectorySummary {
                task: String::new(),
                steps: 0,
                succeeded: 0,
                failed: 0,
                finished: false,
                key_points: vec![],
            };
        }
        let steps: usize = trajectories.iter().map(|t| t.steps.len()).sum();
        let succeeded: usize = trajectories
            .iter()
            .flat_map(|t| t.steps.iter())
            .filter(|s| s.outcome == StepOutcome::Success)
            .count();
        let failed: usize = trajectories
            .iter()
            .flat_map(|t| t.steps.iter())
            .filter(|s| s.outcome == StepOutcome::Error)
            .count();
        let finished = trajectories.iter().any(|t| t.finished);
        // 关键点:成功工具名去重 + 失败信息。
        let mut tools: Vec<String> = trajectories
            .iter()
            .flat_map(|t| t.steps.iter())
            .filter_map(|s| s.tool.clone())
            .collect();
        tools.sort();
        tools.dedup();
        let mut key_points: Vec<String> = tools.iter().map(|t| format!("used tool {t}")).collect();
        for traj in trajectories {
            for step in &traj.steps {
                if let Some(error) = &step.error {
                    key_points.push(format!("failure: {error}"));
                }
            }
        }
        key_points.dedup();
        TrajectorySummary {
            task: trajectories[0].task.clone(),
            steps,
            succeeded,
            failed,
            finished,
            key_points,
        }
    }

    fn inject(&self, task: &str, trajectories: &[Trajectory]) -> MemoryInjection {
        let memories = self.retrieve(task, 5);
        let summary = if trajectories.is_empty() {
            None
        } else {
            Some(self.summarize(trajectories))
        };
        let mut parts: Vec<String> = Vec::new();
        if let Some(summary) = &summary {
            parts.push(format!(
                "Previous experience (task '{}'): {} steps, {} succeeded, {} failed, finished={}",
                summary.task, summary.steps, summary.succeeded, summary.failed, summary.finished
            ));
            for point in &summary.key_points {
                parts.push(format!("- {point}"));
            }
        }
        for memory in &memories {
            parts.push(format!("Memory[{}]: {}", memory.task, memory.content));
        }
        let injected_text = if parts.is_empty() {
            String::new()
        } else {
            format!("Relevant prior context:\n{}", parts.join("\n"))
        };
        MemoryInjection {
            memories: memories.iter().map(|m| m.content.clone()).collect(),
            summary,
            injected_text,
        }
    }
}

/// context-evolver 插件:提供 memory-evolver seam。
pub struct ContextEvolverPlugin {
    dir: PathBuf,
}

impl ContextEvolverPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for ContextEvolverPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-context-evolver"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MEMORY_EVOLVER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service: std::sync::Arc<dyn MemoryEvolver> =
            std::sync::Arc::new(JsonTaskMemoryService::open(self.dir.clone()).map_err(|e| {
                PluginError::Apply {
                    plugin: self.name(),
                    message: e.0,
                }
            })?);
        Ok(vec![ctx.register(MEMORY_EVOLVER, service)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::context_evolver::MemoryEvolver;
    use ah_contracts::evolving::{StepOutcome, TrajectoryStep};
    use ah_contracts::keys::MEMORY_EVOLVER;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ContextEvolverPlugin::new(root.join("memories"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn trajectory(task: &str, tools: &[&str], errors: &[&str]) -> Trajectory {
        let steps = tools
            .iter()
            .map(|tool| TrajectoryStep {
                seq: 0,
                action: format!("use {tool}"),
                tool: Some(tool.to_string()),
                outcome: StepOutcome::Success,
                error: None,
                budget_used: 1,
            })
            .chain(errors.iter().map(|error| TrajectoryStep {
                seq: 0,
                action: "failed step".to_string(),
                tool: None,
                outcome: StepOutcome::Error,
                error: Some(error.to_string()),
                budget_used: 1,
            }))
            .collect();
        Trajectory {
            task: task.to_string(),
            steps,
            finished: errors.is_empty(),
        }
    }

    #[test]
    fn save_retrieve_persists_and_scores() {
        let root = std::env::temp_dir().join(format!("ah-ce-save-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let evolver = ctx
            .service::<dyn MemoryEvolver>(&MEMORY_EVOLVER)
            .expect("evolver");

        evolver
            .save(
                "deploy service",
                "use systemd to keep the process alive",
                vec!["ops".to_string()],
            )
            .expect("save");
        evolver
            .save(
                "write docs",
                "document the API endpoints",
                vec!["docs".to_string()],
            )
            .expect("save2");

        let hits = evolver.retrieve("deploy", 5);
        assert_eq!(hits.len(), 1, "keyword match only");
        assert_eq!(hits[0].task, "deploy service");

        // 空 task/content 显式报错。
        assert!(evolver.save("", "x", vec![]).is_err());

        // 持久化跨重开。
        drop(effects);
        let reopened = build_ctx(&root);
        let evolver2 = reopened
            .0
            .service::<dyn MemoryEvolver>(&MEMORY_EVOLVER)
            .expect("evolver2");
        assert_eq!(evolver2.retrieve("docs", 5).len(), 1, "persisted");
        drop(reopened.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn summarize_and_inject_are_deterministic() {
        let root = std::env::temp_dir().join(format!("ah-ce-sum-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let evolver = ctx
            .service::<dyn MemoryEvolver>(&MEMORY_EVOLVER)
            .expect("evolver");

        evolver
            .save(
                "deploy service",
                "remember to use systemd",
                vec!["ops".to_string()],
            )
            .expect("save");

        let trajectories = vec![trajectory(
            "deploy service",
            &["run_shell", "read_file"],
            &["permission denied"],
        )];
        let summary = evolver.summarize(&trajectories);
        assert_eq!(summary.steps, 3);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 1);
        assert!(!summary.finished);
        assert!(summary.key_points.iter().any(|p| p.contains("run_shell")));
        assert!(
            summary
                .key_points
                .iter()
                .any(|p| p.contains("permission denied"))
        );

        let injection = evolver.inject("deploy", &trajectories);
        assert!(!injection.injected_text.is_empty(), "injection text");
        assert!(injection.injected_text.contains("Previous experience"));
        assert!(
            injection.injected_text.contains("systemd"),
            "retrieved memory injected"
        );

        // 空轨迹:无摘要,但记忆仍在。
        let injection2 = evolver.inject("deploy", &[]);
        assert!(injection2.summary.is_none());
        assert!(!injection2.injected_text.is_empty());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
