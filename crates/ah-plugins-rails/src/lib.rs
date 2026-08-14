//! # ah-plugins-rails
//!
//! 真实 rails:注册为 tools/pre-execute waterfall 监听器。
//! 当前实现:
//! - ShellGuardRail:拒绝 run_shell 工具执行危险命令模式(rm -rf / mkfs / dd if= / fork bomb)。

use ah_contracts::prelude::Effect;
use ah_contracts::tools::{ToolDecision, ToolInvocation};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 危险命令模式(真实、保守的阻止清单)。
pub const DANGEROUS_PATTERNS: &[&str] = &["rm -rf", "mkfs", "dd if=", ":(){"];

/// Shell 守卫 rail:拦截 run_shell 的危险命令。
pub struct ShellGuardRailPlugin;

/// 判断 run_shell 请求是否命中危险模式。
pub fn is_dangerous(command: &str) -> Option<&'static str> {
    DANGEROUS_PATTERNS
        .iter()
        .find(|pattern| command.contains(**pattern))
        .copied()
}

impl Plugin for ShellGuardRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let effect = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |event, decision, next| async move {
                if event.name == "run_shell"
                    && let Some(command) = decision.arguments.get("command").and_then(Value::as_str)
                    && let Some(pattern) = is_dangerous(command)
                {
                    return ToolDecision::deny(
                        decision.arguments,
                        format!("dangerous command pattern: {pattern}"),
                    );
                }
                next.next(decision).await
            },
        );
        Ok(vec![effect])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TOOLS;
    use ah_contracts::tools::ToolRegistry;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            Arc::new(ShellGuardRailPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn shell_guard_allows_safe_command() {
        let root = std::env::temp_dir().join(format!("ah-rails-safe-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let output = registry
            .invoke("run_shell", json!({ "command": "echo", "args": ["safe"] }))
            .await
            .expect("safe command should run");
        assert_eq!(output["exit_code"], 0);
        assert!(
            output["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("safe")
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn shell_guard_blocks_dangerous_command() {
        let root = std::env::temp_dir().join(format!("ah-rails-danger-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let error = registry
            .invoke("run_shell", json!({ "command": "rm -rf /" }))
            .await
            .expect_err("dangerous command must be blocked");
        assert!(error.0.contains("dangerous command pattern"));
        assert!(error.0.contains("rm -rf"));

        // 拒绝后不产生副作用:workspace 里什么都没变(工具未执行)。
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn shell_guard_does_not_affect_other_tools() {
        let root = std::env::temp_dir().join(format!("ah-rails-fs-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        let written = registry
            .invoke("write_file", json!({ "path": "ok.txt", "content": "x" }))
            .await
            .expect("write_file should pass");
        assert_eq!(written["written"], 1);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dangerous_pattern_detection() {
        assert_eq!(is_dangerous("rm -rf /"), Some("rm -rf"));
        assert_eq!(is_dangerous("echo hi"), None);
        assert_eq!(is_dangerous("mkfs.ext4 /dev/sda"), Some("mkfs"));
    }
}
