//! ci seam:CI gate 运行器(auto_harness 基建)。
//!
//! 在指定工作目录以真实子进程运行门禁命令(lint/test/type-check),带回
//! 通过与否、输出、耗时;超时强杀。门禁命令由调用方(编排)配置。

use async_trait::async_trait;
use std::path::PathBuf;

use crate::seam::Seam;

/// 一次门禁请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CiGateRequest {
    /// 门禁名(如 "lint")。
    pub name: String,
    /// 命令(argv 形式,如 ["cargo","clippy","--workspace"])。
    pub command: Vec<String>,
    /// 工作目录;None 用当前目录。
    pub cwd: Option<PathBuf>,
    /// 超时毫秒;None 用实现默认。
    pub timeout_ms: Option<u64>,
}

/// 门禁结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CiGateResult {
    pub name: String,
    /// exit 0 且未超时。
    pub passed: bool,
    /// stdout+stderr 合并输出。
    pub output: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

/// ci 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiError(pub String);

impl core::fmt::Display for CiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CiError {}

/// ci Seam(Service Definition):真实子进程门禁。
#[async_trait]
pub trait CiGateRunner: Seam {
    /// 运行门禁命令并返回真实结果;命令缺失显式报错。
    async fn run_gate(&self, request: CiGateRequest) -> Result<CiGateResult, CiError>;
}
