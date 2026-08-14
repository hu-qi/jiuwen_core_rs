//! code seam:代码执行(真实解释器子进程)。
//!
//! 对应 openjiuwen 的 code 能力:在隔离 scratch 目录写入代码文件,
//! 以真实解释器(python3)执行,带回超时/退出码/输出;执行后清理文件。

use async_trait::async_trait;

use crate::seam::Seam;

/// 代码执行请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CodeExecRequest {
    /// 语言标识(如 "python3");不支持时显式报错。
    pub language: String,
    pub code: String,
    /// 超时毫秒;None 用实现默认。
    pub timeout_ms: Option<u64>,
}

/// 代码执行结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CodeExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// 是否因超时被杀。
    pub timed_out: bool,
    pub duration_ms: u64,
}

/// code 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeError(pub String);

impl core::fmt::Display for CodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodeError {}

/// code Seam(Service Definition):解释器执行。
#[async_trait]
pub trait CodeProvider: Seam {
    /// 支持的语言标识。
    fn supported_languages(&self) -> Vec<String>;

    /// 执行代码(隔离 scratch 目录;超时强杀;输出与退出码真实)。
    async fn execute(&self, request: CodeExecRequest) -> Result<CodeExecResult, CodeError>;
}
