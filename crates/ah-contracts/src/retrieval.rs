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
///
/// 两条检索路径:词法(retrieve,BM25 风格)与向量(retrieve_vector,
/// 余弦相似度)。默认 embedding 为确定性本地向量；插件可注入外部
/// OpenAI-compatible embedding backend 和 KV-backed 向量索引。
pub trait RetrievalProvider: Seam {
    /// 摄入一篇文档(自动分块);同 doc_id 覆盖。
    fn ingest(&self, doc_id: &str, text: &str, metadata: Value) -> Result<(), RetrievalError>;

    /// 词法检索:与查询最相关的 k 个 chunk(按相关性降序)。
    fn retrieve(&self, query: &str, k: usize) -> Vec<RetrievalHit>;

    /// 向量检索:查询 embedding 与 chunk 向量的余弦相似度。
    fn retrieve_vector(&self, query: &str, k: usize) -> Vec<RetrievalHit>;

    /// 向量检索的可失败版本;外部 embedding 后端的网络或响应错误必须传播。
    fn retrieve_vector_checked(
        &self,
        query: &str,
        k: usize,
    ) -> Result<Vec<RetrievalHit>, RetrievalError> {
        Ok(self.retrieve_vector(query, k))
    }

    /// embedding 的可失败版本;默认兼容确定性本地 provider。
    fn embedding_checked(&self, text: &str) -> Result<Vec<f64>, RetrievalError> {
        Ok(self.embedding(text))
    }

    /// embedding 向量;具体 provider 可使用本地确定性算法或外部模型。
    fn embedding(&self, text: &str) -> Vec<f64>;

    /// 删除一篇文档。
    fn remove(&self, doc_id: &str) -> Result<(), RetrievalError>;

    /// 已摄入文档 id。
    fn doc_ids(&self) -> Vec<String>;
}
