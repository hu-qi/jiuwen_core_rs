//! # ah-plugins-rails
//!
//! 真实 rails:注册为 tools/pre-execute waterfall 监听器。
//! 当前实现:
//! - ShellGuardRail:拒绝 run_shell 工具执行危险命令模式(rm -rf / mkfs / dd if= / fork bomb);
//! - PathGuardRail:拒绝 fs 工具(path 参数)使用绝对路径或 .. 逃逸;
//! - ToolBudgetRail:限制工具调用总次数,超限拒绝。

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

/// 判断路径是否逃逸 workspace(绝对路径或含 .. 段)。
pub fn path_escapes_workspace(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    path.split('/').any(|segment| segment == "..")
}

/// 带 path 参数的 fs 工具(PathGuard 作用域)。
pub const PATH_TOOLS: &[&str] = &["read_file", "write_file", "list_dir", "remove_file"];

/// 路径守卫 rail:拦截 fs 工具对 workspace 外路径的访问。
pub struct PathGuardRailPlugin;

impl Plugin for PathGuardRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-path"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let effect = ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(
            |event, decision, next| async move {
                let path = decision
                    .arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if PATH_TOOLS.contains(&event.name.as_str())
                    && let Some(path) = path
                    && path_escapes_workspace(&path)
                {
                    return ToolDecision::deny(
                        decision.arguments,
                        format!("path escapes workspace: {path}"),
                    );
                }
                next.next(decision).await
            },
        );
        Ok(vec![effect])
    }
}

/// 工具预算 rail:限制工具调用总次数(真实计数,跨会话累计)。
pub struct ToolBudgetRailPlugin {
    max_calls: usize,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ToolBudgetRailPlugin {
    /// 以调用上限创建(真实原子计数)。
    pub fn new(max_calls: usize) -> Self {
        Self {
            max_calls,
            calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl Plugin for ToolBudgetRailPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-rails-budget"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        Vec::new()
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let max_calls = self.max_calls;
        let calls = self.calls.clone();
        let effect =
            ctx.on_waterfall::<ToolInvocation, ToolDecision, _, _>(move |event, decision, next| {
                let calls = calls.clone();
                async move {
                    let used = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if used > max_calls {
                        return ToolDecision::deny(
                            decision.arguments,
                            format!("tool budget exceeded ({max_calls})"),
                        );
                    }
                    let _ = event;
                    next.next(decision).await
                }
            });
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

    #[test]
    fn path_escape_detection() {
        assert!(
            path_escapes_workspace("/etc/passwd"),
            "absolute path escapes"
        );
        assert!(
            path_escapes_workspace("../secret.txt"),
            "parent traversal escapes"
        );
        assert!(
            path_escapes_workspace("a/../../b"),
            "nested traversal escapes"
        );
        assert!(!path_escapes_workspace("ok.txt"));
        assert!(!path_escapes_workspace("a/b/c.txt"));
    }

    #[tokio::test]
    async fn path_guard_blocks_escapes_but_allows_safe() {
        let root = std::env::temp_dir().join(format!("ah-rails-path-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(PathGuardRailPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        // 逃逸路径被 rails 拒绝(工具未执行)。
        let err = registry
            .invoke("read_file", json!({ "path": "/etc/passwd" }))
            .await
            .expect_err("absolute path must be blocked");
        assert!(err.0.contains("path escapes workspace"));
        let err = registry
            .invoke("read_file", json!({ "path": "../secret.txt" }))
            .await
            .expect_err("parent traversal must be blocked");
        assert!(err.0.contains("path escapes workspace"));

        // 安全路径正常执行。
        registry
            .invoke("write_file", json!({ "path": "ok.txt", "content": "x" }))
            .await
            .expect("safe write passes");
        let output = registry
            .invoke("read_file", json!({ "path": "ok.txt" }))
            .await
            .expect("safe read passes");
        assert_eq!(output["content"], "x");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn budget_rail_blocks_after_limit() {
        let root = std::env::temp_dir().join(format!("ah-rails-budget-{}", std::process::id()));
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(ToolBudgetRailPlugin::new(2)),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");

        registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect("call 1 within budget");
        registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect("call 2 within budget");
        let err = registry
            .invoke("list_dir", json!({ "path": "." }))
            .await
            .expect_err("call 3 over budget");
        assert!(err.0.contains("tool budget exceeded"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
