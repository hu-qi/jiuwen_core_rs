//! rerank seam:检索结果重排(对齐 openjiuwen retrieval reranker)。
//!
//! 把两条检索路径(词法 + 向量)的候选合并重排:归一化分数加权融合
//! (lexical_weight × bm25 + vector_weight × cosine),可选多样性惩罚
//! (前 k 个已选 chunk 的重叠惩罚),返回最终排序。确定性可测。

use crate::retrieval::RetrievalHit;
use crate::seam::Seam;

/// 重排配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RerankConfig {
    /// 词法分数权重(默认 0.5)。
    pub lexical_weight: f64,
    /// 向量分数权重(默认 0.5)。
    pub vector_weight: f64,
    /// 多样性惩罚系数(默认 0.1;>0 时对与前 k 个已选结果共享词的候选扣分)。
    pub diversity_penalty: f64,
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            lexical_weight: 0.5,
            vector_weight: 0.5,
            diversity_penalty: 0.1,
        }
    }
}

/// 重排后的命中(带融合分数)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RerankedHit {
    pub doc_id: String,
    pub chunk: String,
    /// 融合分数(0..1)。
    pub score: f64,
}

/// rerank 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankError(pub String);

impl core::fmt::Display for RerankError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RerankError {}

/// rerank Seam(Service Definition):词法+向量融合重排。
pub trait Reranker: Seam {
    /// 融合重排:输入词法/向量候选(各按 score 降序),输出按融合分数降序。
    fn rerank(
        &self,
        lexical: &[RetrievalHit],
        vector: &[RetrievalHit],
        k: usize,
        config: &RerankConfig,
    ) -> Result<Vec<RerankedHit>, RerankError>;
}

/// query-aware rerank Seam:保留原始 query 供外部 reranker 使用。
///
/// `Reranker` 的融合 API 为历史兼容接口，不携带 query；外部模型协议必须
/// 使用本 trait，避免把 query 丢失后伪造 vendor rerank 结果。
pub trait QueryReranker: Seam {
    fn rerank_query(
        &self,
        query: &str,
        candidates: &[RetrievalHit],
        k: usize,
    ) -> Result<Vec<RerankedHit>, RerankError>;
}
