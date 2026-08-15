//! tool-approval seam:渐进工具披露(progressive disclosure)。
//!
//! 工具须先获批才能执行;批准集持久化(JSON 文件)。rails 消费方在
//! tools/pre-execute 拒绝未获批工具。

use crate::seam::Seam;

/// tool-approval 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolApprovalError(pub String);

impl core::fmt::Display for ToolApprovalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolApprovalError {}

/// tool-approval Seam(Service Definition):批准集管理。
pub trait ToolApproval: Seam {
    /// 批准一个工具(真实落盘)。
    fn approve(&self, name: &str) -> Result<(), ToolApprovalError>;

    /// 撤销批准。
    fn revoke(&self, name: &str) -> Result<(), ToolApprovalError>;

    /// 是否已批准。
    fn is_approved(&self, name: &str) -> bool;

    /// 全部已批准工具(排序)。
    fn approved(&self) -> Vec<String>;
}
