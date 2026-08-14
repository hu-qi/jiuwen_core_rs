//! context seam:上下文组装与压缩(压缩/offload/token 预算/reinjection)。
//!
//! 对应 openjiuwen/core 的 context_engine:在 token 预算内组装模型可见上下文,
//! 超出预算时把早期消息压缩为摘要并 reinject 为 system 消息,offload 落盘。

use async_trait::async_trait;

use crate::llm::ChatMessage;
use crate::seam::Seam;
use crate::session::SessionLog;

/// 摘要来源(真实记录,不静默)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarySource {
    /// 确定性摘录(真实截取早期消息)。
    Excerpt,
    /// LLM 总结(注入 LLM seam 且成功时)。
    Llm,
}

/// 压缩出的摘要信息。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContextSummary {
    pub summary: String,
    pub source: SummarySource,
    /// 被压缩掉的早期消息条数。
    pub compressed_messages: usize,
    /// 被压缩内容的估计 token 数。
    pub compressed_tokens: usize,
}

/// 组装结果:预算内的消息序列 + 可选摘要。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledContext {
    /// 进入模型请求的消息(摘要被 reinject 为首条 system 消息)。
    pub messages: Vec<ChatMessage>,
    pub summary: Option<ContextSummary>,
    /// 组装后消息的估计 token 总数。
    pub total_tokens: usize,
    pub budget_tokens: usize,
}

/// context 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextError(pub String);

impl core::fmt::Display for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContextError {}

/// context Seam(Service Definition):token 预算内组装与压缩。
///
/// 消费方(agent 循环/CLI)在构造模型请求前调用 assemble,保证
/// 模型可见上下文不超过预算,同时保留早期信息的摘要(日志即真相:
/// 完整历史始终在会话日志中,压缩只影响模型可见窗口)。
#[async_trait]
pub trait ContextEngine: Seam {
    /// 确定性 token 估计(字符/4 + 词数,下限 1;文档注明为启发式)。
    fn estimate_tokens(&self, text: &str) -> usize;

    /// 在预算内组装上下文:预算充足时原样返回;
    /// 超出时压缩最早消息为摘要并 reinject,返回压缩统计。
    async fn assemble(
        &self,
        session: &dyn SessionLog,
        budget_tokens: usize,
    ) -> Result<AssembledContext, ContextError>;

    /// offload:把压缩出的早期消息持久化为 JSONL,返回落盘路径。
    /// 完整历史仍在会话日志;offload 文件供后续恢复/审计。
    async fn offload(
        &self,
        session: &dyn SessionLog,
        budget_tokens: usize,
    ) -> Result<(ContextSummary, String), ContextError>;
}
