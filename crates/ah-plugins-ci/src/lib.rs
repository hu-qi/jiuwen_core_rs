//! # ah-plugins-ci
//!
//! 真实 CI gate 运行器(auto_harness 基建):在指定工作目录以真实子进程运行
//! 门禁命令,带回通过与否/输出/耗时;超时强杀;命令缺失显式报错。

use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::ci::{CiError, CiGateRequest, CiGateResult, CiGateRunner};
use ah_contracts::keys::CI;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use tokio::process::Command;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实子进程 CI gate 运行器。
pub struct SubprocessCiGateRunner;

impl Seam for SubprocessCiGateRunner {}

#[async_trait]
impl CiGateRunner for SubprocessCiGateRunner {
    async fn run_gate(&self, request: CiGateRequest) -> Result<CiGateResult, CiError> {
        let program = request
            .command
            .first()
            .ok_or_else(|| CiError("empty gate command".to_string()))?;
        let args = &request.command[1..];
        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = &request.cwd {
            command.current_dir(cwd);
        }
        command.stdin(std::process::Stdio::null());

        let started = now_ms();
        let timeout = request.timeout_ms.unwrap_or(120_000);
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(timeout), command.output()).await;

        match result {
            Err(_elapsed) => Ok(CiGateResult {
                name: request.name,
                passed: false,
                output: format!("gate timed out after {timeout}ms"),
                duration_ms: now_ms().saturating_sub(started),
                timed_out: true,
            }),
            Ok(Err(e)) => Err(CiError(format!("spawn failed: {e}"))),
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let joined = if stderr.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("{}\n{stderr}", stdout.trim())
                };
                Ok(CiGateResult {
                    name: request.name,
                    passed: output.status.success(),
                    output: joined,
                    duration_ms: now_ms().saturating_sub(started),
                    timed_out: false,
                })
            }
        }
    }
}

/// ci 插件:提供真实子进程门禁运行器。
pub struct CiPlugin;

impl Plugin for CiPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-ci"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CI]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let runner: std::sync::Arc<dyn CiGateRunner> = std::sync::Arc::new(SubprocessCiGateRunner);
        Ok(vec![ctx.register(CI, runner)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::CI;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(CiPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn passing_gate_reports_success_and_output() {
        let (ctx, effects) = build_ctx();
        let ci = ctx.service::<dyn CiGateRunner>(&CI).expect("ci");
        let result = ci
            .run_gate(CiGateRequest {
                name: "smoke".to_string(),
                command: vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "print('gate ok')".to_string(),
                ],
                cwd: None,
                timeout_ms: Some(5000),
            })
            .await
            .expect("run");
        assert!(result.passed);
        assert!(result.output.contains("gate ok"));
        assert!(!result.timed_out);
        drop(effects);
    }

    #[tokio::test]
    async fn failing_gate_reports_failure() {
        let (ctx, effects) = build_ctx();
        let ci = ctx.service::<dyn CiGateRunner>(&CI).expect("ci");
        let result = ci
            .run_gate(CiGateRequest {
                name: "lint".to_string(),
                command: vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "import sys; sys.exit(1)".to_string(),
                ],
                cwd: None,
                timeout_ms: Some(5000),
            })
            .await
            .expect("run");
        assert!(!result.passed, "non-zero exit -> gate failed");
        drop(effects);
    }

    #[tokio::test]
    async fn timeout_kills_long_gate() {
        let (ctx, effects) = build_ctx();
        let ci = ctx.service::<dyn CiGateRunner>(&CI).expect("ci");
        let result = ci
            .run_gate(CiGateRequest {
                name: "slow".to_string(),
                command: vec![
                    "python3".to_string(),
                    "-c".to_string(),
                    "import time; time.sleep(10)".to_string(),
                ],
                cwd: None,
                timeout_ms: Some(500),
            })
            .await
            .expect("run");
        assert!(result.timed_out, "long gate killed by timeout");
        assert!(!result.passed);
        drop(effects);
    }

    #[tokio::test]
    async fn missing_command_errors_explicitly() {
        let (ctx, effects) = build_ctx();
        let ci = ctx.service::<dyn CiGateRunner>(&CI).expect("ci");
        let err = ci
            .run_gate(CiGateRequest {
                name: "ghost".to_string(),
                command: vec!["no_such_binary_xyz".to_string()],
                cwd: None,
                timeout_ms: Some(1000),
            })
            .await
            .expect_err("missing binary must error");
        assert!(err.0.contains("spawn failed") || err.0.contains("No such file"));
        drop(effects);
    }
}
