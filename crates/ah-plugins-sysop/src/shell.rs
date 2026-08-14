//! 真实本地 shell provider(受限工作目录 + 超时)。

use std::path::PathBuf;
use std::time::Duration;

use ah_contracts::seam::Seam;
use ah_contracts::shell::{ShellError, ShellOutput, ShellProvider};
use async_trait::async_trait;
use tokio::process::Command;

/// 真实本地 shell:在受限工作目录内执行命令,超时终止。
pub struct LocalShellProvider {
    cwd: PathBuf,
}

impl LocalShellProvider {
    /// 以工作目录创建 provider;目录不存在则创建。
    pub fn new(cwd: impl Into<PathBuf>) -> Result<Self, ShellError> {
        let cwd = cwd.into();
        if !cwd.exists() {
            std::fs::create_dir_all(&cwd)
                .map_err(|e| ShellError(format!("create cwd failed: {e}")))?;
        }
        Ok(Self { cwd })
    }
}

impl Seam for LocalShellProvider {}

#[async_trait]
impl ShellProvider for LocalShellProvider {
    fn cwd(&self) -> PathBuf {
        self.cwd.clone()
    }

    async fn run(
        &self,
        command: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<ShellOutput, ShellError> {
        let child = Command::new(command)
            .args(args)
            .current_dir(&self.cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ShellError(format!("spawn {command} failed: {e}")))?;

        // 注意:kill_on_drop(true) 保证超时 drop future 时子进程被终止。
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Err(_) => {
                return Err(ShellError(format!(
                    "command {command} timed out after {timeout:?}"
                )));
            }
            Ok(Err(error)) => return Err(ShellError(format!("wait failed: {error}"))),
            Ok(Ok(output)) => output,
        };

        Ok(ShellOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn provider(tag: &str) -> (LocalShellProvider, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ah-shell-{tag}-{}", std::process::id()));
        let provider = LocalShellProvider::new(&dir).expect("temp cwd");
        (provider, dir)
    }

    #[tokio::test]
    async fn runs_real_command_and_captures_output() {
        let (shell, dir) = provider("echo");
        let args = vec!["hello real shell".to_string()];
        let output = shell
            .run("echo", &args, Duration::from_secs(5))
            .await
            .expect("echo");
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello real shell"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn captures_exit_code() {
        let (shell, dir) = provider("exitcode");
        let output = shell
            .run(
                "sh",
                &["-c".to_string(), "exit 3".to_string()],
                Duration::from_secs(5),
            )
            .await
            .expect("sh");
        assert_eq!(output.exit_code, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn times_out_long_command() {
        let (shell, dir) = provider("timeout");
        let result = shell
            .run(
                "sh",
                &["-c".to_string(), "sleep 10".to_string()],
                Duration::from_millis(100),
            )
            .await;
        assert!(matches!(result, Err(ShellError(message)) if message.contains("timed out")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_command_errors() {
        let (shell, dir) = provider("missing");
        let result = shell
            .run("definitely-not-a-command-xyz", &[], Duration::from_secs(5))
            .await;
        assert!(matches!(result, Err(ShellError(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
