//! memory seam:持久化记忆(存储/检索/搜索/删除)。

use crate::seam::Seam;

/// 一条记忆记录。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryRecord {
    pub key: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_ms: u64,
}

/// 记忆错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryError(pub String);

impl core::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MemoryError {}

/// 记忆 Seam(Service Definition):agent 的持久化记忆。
pub trait MemoryProvider: Seam {
    /// 存储一条记忆(同 key 覆盖),返回记录。
    fn store(
        &self,
        key: &str,
        content: &str,
        tags: Vec<String>,
    ) -> Result<MemoryRecord, MemoryError>;

    /// 按 key 取回。
    fn retrieve(&self, key: &str) -> Option<MemoryRecord>;

    /// 搜索:内容或标签含查询词。
    fn search(&self, query: &str) -> Vec<MemoryRecord>;

    /// 全部记忆(按 key 排序)。
    fn list(&self) -> Vec<MemoryRecord>;

    /// 删除一条记忆。
    fn remove(&self, key: &str) -> Result<(), MemoryError>;
}
