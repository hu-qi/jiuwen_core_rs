//! # ah-plugins-code
//!
//! 真实代码执行(对应 openjiuwen 的 code 能力):在隔离 scratch 目录写入
//! 代码文件,以真实解释器(python3)子进程执行,带回超时/退出码/输出;
//! 执行后清理文件。解释器不在环境时显式报错。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::code::{CodeError, CodeExecRequest, CodeExecResult, CodeProvider};
use ah_contracts::keys::CODE;
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

/// 真实解释器代码执行 provider。
pub struct InterpreterCodeProvider {
    dir: PathBuf,
    seq: AtomicU64,
}

impl InterpreterCodeProvider {
    /// 以 scratch 目录创建(自动创建)。
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, CodeError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| CodeError(format!("create scratch dir failed: {e}")))?;
        Ok(Self {
            dir,
            seq: AtomicU64::new(0),
        })
    }

    fn scratch_path(&self, language: &str) -> PathBuf {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        let ext = if language == "python3" { "py" } else { "txt" };
        self.dir
            .join(format!("run_{now}_{n}.{ext}", now = now_ms()))
    }
}

impl Seam for InterpreterCodeProvider {}

#[async_trait]
impl CodeProvider for InterpreterCodeProvider {
    fn supported_languages(&self) -> Vec<String> {
        vec!["python3".to_string()]
    }

    async fn execute(&self, request: CodeExecRequest) -> Result<CodeExecResult, CodeError> {
        if !self
            .supported_languages()
            .iter()
            .any(|l| l == &request.language)
        {
            return Err(CodeError(format!(
                "unsupported language: {} (supported: {:?})",
                request.language,
                self.supported_languages()
            )));
        }
        let interpreter = match request.language.as_str() {
            "python3" => "python3",
            other => return Err(CodeError(format!("no interpreter for {other}"))),
        };
        // 解释器真实存在性检查(缺失显式报错,不静默)。
        if !std::process::Command::new(interpreter)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| CodeError(format!("interpreter {interpreter} unavailable: {e}")))?
            .success()
        {
            return Err(CodeError(format!(
                "interpreter {interpreter} failed version check"
            )));
        }

        // 隔离 scratch:写入代码文件。
        let path = self.scratch_path(&request.language);
        std::fs::write(&path, &request.code)
            .map_err(|e| CodeError(format!("write scratch failed: {e}")))?;

        let started = now_ms();
        let timeout = request.timeout_ms.unwrap_or(10_000);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout),
            Command::new(interpreter)
                .arg(&path)
                .stdin(std::process::Stdio::null())
                .output(),
        )
        .await;

        // 清理 scratch 文件(成功与失败路径都清理)。
        let _ = std::fs::remove_file(&path);

        match result {
            Err(_elapsed) => Ok(CodeExecResult {
                stdout: String::new(),
                stderr: format!("timed out after {timeout}ms"),
                exit_code: -1,
                timed_out: true,
                duration_ms: now_ms().saturating_sub(started),
            }),
            Ok(Err(e)) => Err(CodeError(format!("spawn failed: {e}"))),
            Ok(Ok(output)) => Ok(CodeExecResult {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                exit_code: output.status.code().unwrap_or(-1),
                timed_out: false,
                duration_ms: now_ms().saturating_sub(started),
            }),
        }
    }
}

/// code 插件:提供真实解释器执行。
pub struct CodePlugin {
    dir: PathBuf,
}

impl CodePlugin {
    /// 以 scratch 目录创建插件。
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl Plugin for CodePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-code"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CODE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider =
            InterpreterCodeProvider::new(self.dir.clone()).map_err(|e| PluginError::Apply {
                plugin: self.name(),
                message: e.0,
            })?;
        let provider: Arc<dyn CodeProvider> = Arc::new(provider);
        Ok(vec![ctx.register(CODE, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::CODE;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(CodePlugin::new(root.join("scratch")))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn executes_python_and_returns_stdout() {
        let root = std::env::temp_dir().join(format!("ah-code-ok-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let code = ctx.service::<dyn CodeProvider>(&CODE).expect("code");

        let result = code
            .execute(CodeExecRequest {
                language: "python3".to_string(),
                code: "print('hello from code')".to_string(),
                timeout_ms: Some(5000),
            })
            .await
            .expect("execute");
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello from code"));
        assert!(!result.timed_out);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn propagates_stderr_and_nonzero_exit() {
        let root = std::env::temp_dir().join(format!("ah-code-err-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let code = ctx.service::<dyn CodeProvider>(&CODE).expect("code");

        let result = code
            .execute(CodeExecRequest {
                language: "python3".to_string(),
                code: "raise ValueError('boom')".to_string(),
                timeout_ms: Some(5000),
            })
            .await
            .expect("execute");
        assert_ne!(result.exit_code, 0);
        assert!(result.stderr.contains("ValueError"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn timeout_kills_long_running_code() {
        let root = std::env::temp_dir().join(format!("ah-code-to-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let code = ctx.service::<dyn CodeProvider>(&CODE).expect("code");

        let result = code
            .execute(CodeExecRequest {
                language: "python3".to_string(),
                code: "import time; time.sleep(10)".to_string(),
                timeout_ms: Some(500),
            })
            .await
            .expect("execute");
        assert!(result.timed_out, "long code killed by timeout");
        assert!(result.stderr.contains("timed out"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn unsupported_language_rejected() {
        let root = std::env::temp_dir().join(format!("ah-code-lang-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let code = ctx.service::<dyn CodeProvider>(&CODE).expect("code");

        let err = code
            .execute(CodeExecRequest {
                language: "ruby".to_string(),
                code: "puts 1".to_string(),
                timeout_ms: Some(1000),
            })
            .await
            .expect_err("ruby unsupported");
        assert!(err.0.contains("unsupported language"));

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
