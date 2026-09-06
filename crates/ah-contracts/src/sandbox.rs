//! sandbox seam:策略化沙箱(路径/命令访问决策)。
//!
//! 对应 openjiuwen 的 sandbox:本地策略后端为 sandbox.json(允许路径前缀、
//! 拒绝命令模式、绝对路径开关),消费方为 tools/pre-execute rail;远程
//! JSON HTTP sandbox provider 由插件提供。

use crate::seam::Seam;

/// 沙箱策略(文件后端,真实持久化)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SandboxPolicy {
    /// 允许的路径前缀(相对 workspace;空 = 仅默认限制)。
    pub allowed_path_prefixes: Vec<String>,
    /// 拒绝的命令模式(子串匹配)。
    pub denied_command_patterns: Vec<String>,
    /// 是否允许绝对路径访问。
    pub allow_absolute_paths: bool,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            allowed_path_prefixes: vec![],
            denied_command_patterns: vec!["rm -rf".to_string(), "mkfs".to_string()],
            allow_absolute_paths: false,
        }
    }
}

/// 文件系统访问决策。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FsDecision {
    pub allow: bool,
    pub reason: String,
}

/// 命令执行决策。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandDecision {
    pub allow: bool,
    pub reason: String,
}

/// 沙箱错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxError(pub String);

impl core::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SandboxError {}

/// sandbox Seam(Service Definition):策略驱动的访问决策。
pub trait SandboxProvider: Seam {
    /// 文件系统访问决策(绝对路径/逃逸/前缀允许)。
    fn check_fs(&self, path: &str) -> Result<FsDecision, SandboxError>;

    /// 命令执行决策(拒绝模式匹配)。
    fn check_command(&self, command: &str) -> Result<CommandDecision, SandboxError>;

    /// 当前生效策略。
    fn policy(&self) -> Result<SandboxPolicy, SandboxError>;

    /// 更新策略(真实落盘)。
    fn set_policy(&self, policy: SandboxPolicy) -> Result<(), SandboxError>;
}
