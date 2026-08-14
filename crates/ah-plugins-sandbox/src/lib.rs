//! # ah-plugins-sandbox
//!
//! 真实策略化沙箱(对应 openjiuwen sandbox 的本地基础):
//!
//! - FileSandboxProvider:sandbox.json 持久化策略(允许路径前缀/拒绝命令模式/
//!   绝对路径开关),check_fs/check_command 真实决策;
//! - SandboxRailPlugin:消费方,挂在 tools/pre-execute waterfall 上,
//!   fs 工具查路径、run_shell 查命令。
//!
//! 远程沙箱(容器/VM)留待后续,文档注明。

use std::path::PathBuf;
use std::sync::Mutex;

use ah_contracts::keys::SANDBOX;
use ah_contracts::prelude::Effect;
use ah_contracts::sandbox::{
    CommandDecision, FsDecision, SandboxError, SandboxPolicy, SandboxProvider,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{ToolDecision, ToolInvocation};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 真实文件后端沙箱策略。
pub struct FileSandboxProvider {
    dir: PathBuf,
    policy: Mutex<SandboxPolicy>,
}

impl FileSandboxProvider {
    /// 打开(或创建)策略文件;sandbox.json 缺失时写入默认策略。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, SandboxError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| SandboxError(format!("create sandbox dir failed: {e}")))?;
        let path = dir.join("sandbox.json");
        let policy = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| SandboxError(format!("read policy: {e}")))?;
            serde_json::from_str(&text).map_err(|e| SandboxError(format!("parse policy: {e}")))?
        } else {
            let default = SandboxPolicy::default();
            std::fs::write(&path, serde_json::to_string_pretty(&default).unwrap())
                .map_err(|e| SandboxError(format!("write default policy: {e}")))?;
            default
        };
        Ok(Self {
            dir,
            policy: Mutex::new(policy),
        })
    }

    fn persist(&self, policy: &SandboxPolicy) -> Result<(), SandboxError> {
        std::fs::write(
            self.dir.join("sandbox.json"),
            serde_json::to_string_pretty(policy)
                .map_err(|e| SandboxError(format!("serialize policy: {e}")))?,
        )
        .map_err(|e| SandboxError(format!("write policy: {e}")))
    }
}

impl Seam for FileSandboxProvider {}

impl SandboxProvider for FileSandboxProvider {
    fn check_fs(&self, path: &str) -> Result<FsDecision, SandboxError> {
        let policy = self.policy.lock().unwrap();
        if !policy.allow_absolute_paths && path.starts_with('/') {
            return Ok(FsDecision {
                allow: false,
                reason: format!("absolute path not allowed: {path}"),
            });
        }
        if path.split('/').any(|seg| seg == "..") {
            return Ok(FsDecision {
                allow: false,
                reason: format!("path escapes workspace: {path}"),
            });
        }
        if !policy.allowed_path_prefixes.is_empty() {
            let allowed = policy
                .allowed_path_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix));
            if !allowed {
                return Ok(FsDecision {
                    allow: false,
                    reason: format!("path outside allowed prefixes: {path}"),
                });
            }
        }
        Ok(FsDecision {
            allow: true,
            reason: "allowed".to_string(),
        })
    }

    fn check_command(&self, command: &str) -> Result<CommandDecision, SandboxError> {
        let policy = self.policy.lock().unwrap();
        if let Some(pattern) = policy
            .denied_command_patterns
            .iter()
            .find(|p| command.contains(p.as_str()))
        {
            return Ok(CommandDecision {
                allow: false,
                reason: format!("denied command pattern: {pattern}"),
            });
        }
        Ok(CommandDecision {
            allow: true,
            reason: "allowed".to_string(),
        })
    }

    fn policy(&self) -> Result<SandboxPolicy, SandboxError> {
        Ok(self.policy.lock().unwrap().clone())
    }

    fn set_policy(&self, policy: SandboxPolicy) -> Result<(), SandboxError> {
        self.persist(&policy)?;
        *self.policy.lock().unwrap() = policy;
        Ok(())
    }
}

/// 带 path 参数的 fs 工具(沙箱检查作用域)。
pub const SANDBOX_PATH_TOOLS: &[&str] = &["read_file", "write_file", "list_dir", "remove_file"];

/// 沙箱 rail:消费 sandbox seam,在 tools/pre-execute 决策。
pub struct SandboxRailPlugin;

impl Plugin for SandboxRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-sandbox-rail"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let sandbox = ctx
            .service::<dyn SandboxProvider>(&SANDBOX)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "sandbox seam not registered".to_string(),
            })?;
        let effect =
            ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(move |event, decision, next| {
                let sandbox = sandbox.clone();
                async move {
                    if SANDBOX_PATH_TOOLS.contains(&event.name.as_str()) {
                        let path = decision
                            .arguments
                            .get("path")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Ok(fs) = sandbox.check_fs(path)
                            && !fs.allow
                        {
                            return ToolDecision::deny(decision.arguments, fs.reason);
                        }
                    }
                    if event.name == "run_shell"
                        && let Some(command) =
                            decision.arguments.get("command").and_then(Value::as_str)
                        && let Ok(cmd) = sandbox.check_command(command)
                        && !cmd.allow
                    {
                        return ToolDecision::deny(decision.arguments, cmd.reason);
                    }
                    next.next(decision).await
                }
            });
        Ok(vec![effect])
    }
}

/// sandbox 插件:提供策略化沙箱 seam。
pub struct SandboxPlugin {
    dir: PathBuf,
}

impl SandboxPlugin {
    /// 以策略目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for SandboxPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-sandbox"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SANDBOX]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider =
            FileSandboxProvider::open(self.dir.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let provider: std::sync::Arc<dyn SandboxProvider> = std::sync::Arc::new(provider);
        Ok(vec![ctx.register(SANDBOX, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{SANDBOX, TOOLS};
    use ah_contracts::tools::ToolRegistry;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            Arc::new(SandboxPlugin::new(root.join("sandbox"))),
            Arc::new(SandboxRailPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn default_policy_decisions() {
        let root = std::env::temp_dir().join(format!("ah-sb-pol-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let sandbox = ctx
            .service::<dyn SandboxProvider>(&SANDBOX)
            .expect("sandbox");

        assert!(
            !sandbox.check_fs("/etc/passwd").expect("abs").allow,
            "absolute denied"
        );
        assert!(
            !sandbox.check_fs("../secret").expect("esc").allow,
            "escape denied"
        );
        assert!(
            sandbox.check_fs("ok.txt").expect("rel").allow,
            "relative allowed"
        );
        assert!(
            !sandbox.check_command("rm -rf /").expect("rm").allow,
            "rm -rf denied"
        );
        assert!(sandbox.check_command("echo hi").expect("echo").allow);

        // 默认策略真实落盘。
        assert!(root.join("sandbox").join("sandbox.json").exists());

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_policy_persists_and_takes_effect() {
        let root = std::env::temp_dir().join(format!("ah-sb-set-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let sandbox = ctx
            .service::<dyn SandboxProvider>(&SANDBOX)
            .expect("sandbox");

        let mut policy = SandboxPolicy {
            allowed_path_prefixes: vec!["safe/".to_string()],
            ..Default::default()
        };
        policy.denied_command_patterns.push("danger".to_string());
        sandbox.set_policy(policy).expect("set");

        // 前缀外路径被拒;前缀内放行。
        assert!(
            !sandbox.check_fs("other/x").expect("x").allow,
            "outside prefix denied"
        );
        assert!(
            sandbox.check_fs("safe/x").expect("y").allow,
            "inside prefix allowed"
        );
        assert!(
            !sandbox.check_command("run danger").expect("d").allow,
            "custom pattern denied"
        );

        drop(effects);
        // 重开:策略从文件恢复。
        let reopened = FileSandboxProvider::open(root.join("sandbox")).expect("reopen");
        let restored = reopened.policy().expect("policy");
        assert_eq!(restored.allowed_path_prefixes, vec!["safe/".to_string()]);
        assert!(
            restored
                .denied_command_patterns
                .contains(&"danger".to_string())
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rail_enforces_policy_before_execution() {
        let root = std::env::temp_dir().join(format!("ah-sb-rail-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let err = registry
            .invoke("read_file", json!({ "path": "/etc/passwd" }))
            .await
            .expect_err("absolute path blocked by sandbox rail");
        assert!(err.0.contains("absolute path not allowed"));
        let err = registry
            .invoke("run_shell", json!({ "command": "rm -rf /" }))
            .await
            .expect_err("dangerous command blocked by sandbox rail");
        assert!(err.0.contains("denied command pattern"));
        // 安全操作放行。
        registry
            .invoke("write_file", json!({ "path": "ok.txt", "content": "x" }))
            .await
            .expect("safe write passes");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
