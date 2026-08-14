//! fs seam:文件系统能力。

use std::path::PathBuf;

use crate::seam::Seam;

/// 文件系统错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsError(pub String);

impl core::fmt::Display for FsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FsError {}

/// 文件系统 Seam(Service Definition)。
///
/// 所有路径为相对 workspace root 的路径;实现方必须做路径约束,
/// 拒绝任何逃逸 root 的访问。
pub trait FsProvider: Seam {
    /// workspace 根目录(绝对路径)。
    fn root(&self) -> PathBuf;

    /// 读取相对路径下的文件内容。
    fn read(&self, rel: &str) -> Result<Vec<u8>, FsError>;

    /// 写入相对路径(自动创建父目录)。
    fn write(&self, rel: &str, content: &[u8]) -> Result<(), FsError>;

    /// 列出相对路径下的条目名。
    fn list(&self, rel: &str) -> Result<Vec<String>, FsError>;

    /// 删除相对路径(文件或空目录)。
    fn remove(&self, rel: &str) -> Result<(), FsError>;

    /// 相对路径是否存在。
    fn exists(&self, rel: &str) -> bool;
}
