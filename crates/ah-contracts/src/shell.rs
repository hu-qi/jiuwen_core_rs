//! shell seam:命令执行能力。

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use crate::seam::Seam;

/// shell 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellError(pub String);

impl core::fmt::Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ShellError {}

/// 命令执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// shell Seam(Service Definition)。
///
/// 在受约束的工作目录内执行命令;实现方负责超时与输出捕获。
#[async_trait]
pub trait ShellProvider: Seam {
    /// 工作目录。
    fn cwd(&self) -> PathBuf;

    /// 执行命令,超时返回错误。
    async fn run(
        &self,
        command: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<ShellOutput, ShellError>;
}
