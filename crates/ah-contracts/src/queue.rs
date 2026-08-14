//! queue seam:消息队列(发布/消费/积压)。
//!
//! 对应 openjiuwen 的 messager/queue:本地真实队列为日志+游标模型
//! (每 channel 一个 append-only JSONL + 消费游标),外部传输
//! (Pulsar/ZMQ ROUTER-DEALER)留待后续,文档注明。

use serde_json::Value;

use crate::seam::Seam;

/// 一条队列消息。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueueMessage {
    pub channel: String,
    /// 单调递增序号(按 channel)。
    pub seq: u64,
    pub payload: Value,
    pub ts_ms: u64,
}

/// 队列错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueError(pub String);

impl core::fmt::Display for QueueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueueError {}

/// queue Seam(Service Definition):发布/消费/积压。
///
/// 消费语义:每次 consume 返回当前游标之后最早一条并推进游标
/// (日志为真相,backlog 始终保留全部消息)。
pub trait MessageQueue: Seam {
    /// 发布一条消息到 channel,返回带序号的完整记录。
    fn publish(&self, channel: &str, payload: Value) -> Result<QueueMessage, QueueError>;

    /// 消费下一条;无未消费消息返回 None。
    fn consume(&self, channel: &str) -> Result<Option<QueueMessage>, QueueError>;

    /// 积压:channel 全部消息(按 seq 升序,含已消费)。
    fn backlog(&self, channel: &str) -> Result<Vec<QueueMessage>, QueueError>;

    /// 已存在消息的 channel 列表。
    fn channels(&self) -> Result<Vec<String>, QueueError>;
}
