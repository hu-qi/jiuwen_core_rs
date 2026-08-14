//! retrieval seam:知识库检索(摄入/检索/删除)。

use serde_json::Value;

use crate::seam::Seam;

/// 一条检索命中。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RetrievalHit {
    pub doc_id: String,
    pub chunk: String,
    pub score: f64,
}

/// 检索错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalError(pub String);

impl core::fmt::Display for RetrievalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RetrievalError {}

/// 检索 Seam(Service Definition):文档摄入与查询。
pub trait RetrievalProvider: Seam {
    /// 摄入一篇文档(自动分块);同 doc_id 覆盖。
    fn ingest(&self, doc_id: &str, text: &str, metadata: Value) -> Result<(), RetrievalError>;

    /// 检索与查询最相关的 k 个 chunk(按相关性降序)。
    fn retrieve(&self, query: &str, k: usize) -> Vec<RetrievalHit>;

    /// 删除一篇文档。
    fn remove(&self, doc_id: &str) -> Result<(), RetrievalError>;

    /// 已摄入文档 id。
    fn doc_ids(&self) -> Vec<String>;
}
