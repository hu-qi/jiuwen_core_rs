//! # ah-plugins-symphony
//!
//! 真实 symphony(按 README 语义):能力注册 + 语义指纹 + 按任务检索 +
//! 可解释编排计划 + 执行(工具调用或 subagent 委派)。注册表 JSONL 持久化。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ah_contracts::keys::{SUBAGENT, SYMPHONY, TOOLS};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::subagent::{SubagentRuntime, SubagentSpec};
use ah_contracts::symphony::{
    Capability, CapabilityFingerprint, ExecutionStep, OrchestrationPlan, Symphony, SymphonyError,
};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// 真实 symphony 引擎。
pub struct SymphonyEngine {
    dir: PathBuf,
    capabilities: Mutex<std::collections::HashMap<String, Capability>>,
    tools: Arc<dyn ToolRegistry>,
    subagent: Arc<dyn SubagentRuntime>,
}

impl SymphonyEngine {
    pub fn open(
        dir: impl Into<PathBuf>,
        tools: Arc<dyn ToolRegistry>,
        subagent: Arc<dyn SubagentRuntime>,
    ) -> Result<Self, SymphonyError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| SymphonyError(format!("create symphony dir: {e}")))?;
        let mut capabilities = std::collections::HashMap::new();
        let path = dir.join("capabilities.jsonl");
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                if let Ok(capability) = serde_json::from_str::<Capability>(line) {
                    capabilities.insert(capability.id.clone(), capability);
                }
            }
        }
        Ok(Self {
            dir,
            capabilities: Mutex::new(capabilities),
            tools,
            subagent,
        })
    }

    fn persist(&self, capability: &Capability) -> Result<(), SymphonyError> {
        let path = self.dir.join("capabilities.jsonl");
        let line = serde_json::to_string(capability)
            .map_err(|e| SymphonyError(format!("serialize capability: {e}")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SymphonyError(format!("open capabilities: {e}")))?;
        use std::io::Write;
        writeln!(file, "{line}").map_err(|e| SymphonyError(format!("append capability: {e}")))?;
        Ok(())
    }
}

impl Seam for SymphonyEngine {}

#[async_trait]
impl Symphony for SymphonyEngine {
    fn register_capability(&self, capability: Capability) -> Result<(), SymphonyError> {
        if capability.id.is_empty() || capability.name.is_empty() {
            return Err(SymphonyError(
                "capability id/name must not be empty".to_string(),
            ));
        }
        let mut map = self.capabilities.lock().unwrap();
        if map.contains_key(&capability.id) {
            return Err(SymphonyError(format!(
                "capability already exists: {}",
                capability.id
            )));
        }
        self.persist(&capability)?;
        map.insert(capability.id.clone(), capability);
        Ok(())
    }

    fn fingerprint(&self, id: &str) -> Option<CapabilityFingerprint> {
        let capability = self.capabilities.lock().unwrap().get(id)?.clone();
        let input_hint: String = capability.description.chars().take(60).collect();
        // 输出提示:描述中与标签共现的关键词。
        let output_hint = capability
            .tags
            .iter()
            .filter(|t| capability.description.contains(t.as_str()))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        Some(CapabilityFingerprint {
            id: capability.id,
            name: capability.name,
            tags: capability.tags,
            input_hint,
            output_hint,
        })
    }

    fn find_capabilities(&self, task: &str) -> Vec<Capability> {
        let lower = task.to_lowercase();
        let mut found: Vec<Capability> = self
            .capabilities
            .lock()
            .unwrap()
            .values()
            .filter(|c| {
                c.tags.iter().any(|t| lower.contains(&t.to_lowercase()))
                    || c.description.to_lowercase().contains(&lower)
                    || lower.contains(&c.name.to_lowercase())
            })
            .cloned()
            .collect();
        found.sort_by(|a, b| a.id.cmp(&b.id));
        found
    }

    fn plan(&self, task: &str) -> Result<OrchestrationPlan, SymphonyError> {
        let candidates = self.find_capabilities(task);
        if candidates.is_empty() {
            return Err(SymphonyError("no capability matches the task".to_string()));
        }
        let steps: Vec<ExecutionStep> = candidates
            .iter()
            .map(|c| ExecutionStep {
                capability_id: c.id.clone(),
                name: c.name.clone(),
                detail: c.description.clone(),
            })
            .collect();
        let rationale = format!(
            "selected {} capabilities matching the task (tags/keywords)",
            candidates.len()
        );
        Ok(OrchestrationPlan {
            task: task.to_string(),
            steps,
            rationale,
        })
    }

    async fn execute(&self, plan: &OrchestrationPlan, task: &str) -> Result<Value, SymphonyError> {
        let mut outputs = Vec::new();
        for step in &plan.steps {
            let capability = self
                .capabilities
                .lock()
                .unwrap()
                .get(&step.capability_id)
                .cloned()
                .ok_or_else(|| SymphonyError(format!("capability gone: {}", step.capability_id)))?;
            if let Some(tool) = &capability.tool {
                let output = self
                    .tools
                    .invoke(tool, json!({ "task": task }))
                    .await
                    .map_err(|e| SymphonyError(format!("tool {tool} failed: {e}")))?;
                outputs.push(json!({ "capability": capability.id, "output": output }));
            } else {
                // 无工具:subagent 委派。
                let result = self
                    .subagent
                    .run(SubagentSpec {
                        id: format!("symphony-{}-{}", capability.id, task.len()),
                        task: format!("{}: {task}", capability.description),
                        context: Some(capability.description.clone()),
                        budget: Some(4),
                        allowed_tools: None,
                    })
                    .await
                    .map_err(|e| SymphonyError(format!("subagent failed: {e}")))?;
                outputs.push(json!({ "capability": capability.id, "answer": result.answer }));
            }
        }
        Ok(json!({ "steps": outputs }))
    }
}

/// symphony 插件:注入 tools + subagent,提供 symphony seam。
pub struct SymphonyPlugin {
    dir: PathBuf,
}

impl SymphonyPlugin {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for SymphonyPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-symphony"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SYMPHONY]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS, SUBAGENT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let tools = ctx
            .service::<dyn ToolRegistry>(&TOOLS)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "tools seam not registered".to_string(),
            })?;
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let engine = SymphonyEngine::open(self.dir.clone(), tools, subagent).map_err(|e| {
            PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            }
        })?;
        let engine: Arc<dyn Symphony> = Arc::new(engine);
        Ok(vec![ctx.register(SYMPHONY, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SYMPHONY;
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
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(SymphonyPlugin::new(root.join("symphony"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn capability(
        id: &str,
        name: &str,
        desc: &str,
        tags: &[&str],
        tool: Option<&str>,
    ) -> Capability {
        Capability {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            tool: tool.map(str::to_string),
        }
    }

    #[test]
    fn register_fingerprint_and_find() {
        let root = std::env::temp_dir().join(format!("ah-sy-reg-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let symphony = ctx.service::<dyn Symphony>(&SYMPHONY).expect("symphony");

        symphony
            .register_capability(capability(
                "explore",
                "explore workspace",
                "list and read files in the workspace",
                &["workspace", "files"],
                Some("list_dir"),
            ))
            .expect("register");
        assert!(
            symphony
                .register_capability(capability("explore", "x", "y", &[], None))
                .is_err()
        );

        let fp = symphony.fingerprint("explore").expect("fingerprint");
        assert_eq!(fp.name, "explore workspace");
        assert!(fp.input_hint.contains("list and read"));

        let found = symphony.find_capabilities("workspace files");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "explore");

        // 持久化跨重开。
        drop(effects);
        let reopened = build_ctx(&root);
        let symphony2 = reopened
            .0
            .service::<dyn Symphony>(&SYMPHONY)
            .expect("symphony");
        assert!(symphony2.fingerprint("explore").is_some(), "persisted");
        drop(reopened.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn plan_and_execute_real_steps() {
        let root = std::env::temp_dir().join(format!("ah-sy-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let symphony = ctx.service::<dyn Symphony>(&SYMPHONY).expect("symphony");
        symphony
            .register_capability(capability(
                "explore",
                "explore workspace",
                "list and read files in the workspace",
                &["workspace", "files"],
                Some("list_dir"),
            ))
            .expect("register");

        let plan = symphony.plan("list workspace files").expect("plan");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].capability_id, "explore");

        let output = symphony
            .execute(&plan, "list workspace files")
            .await
            .expect("execute");
        assert!(output.get("steps").is_some(), "real execution output");
        // 无匹配任务 → 编排显式报错。
        assert!(symphony.plan("completely unrelated").is_err());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
