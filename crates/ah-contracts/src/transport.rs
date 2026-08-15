//! transport seam:A2A 风格 agent 传输(JSON-RPC 2.0 over HTTP)。
//!
//! 客户端(JsonRpcTransport)与本地 HTTP 服务端(插件侧 AgentHttpServer)
//! 走真实协议路径:agent/getCard 与 message/send 两个 JSON-RPC 方法。
//! 完整 A2A 规范(SSE/流式/加密传输)留待后续,文档注明。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::seam::Seam;

/// agent 卡(端点元数据)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub skills: Vec<String>,
}

/// agent 消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    /// 角色(user / assistant / system)。
    pub role: String,
    pub content: String,
    /// 消息类型(message / text / …)。
    pub kind: String,
}

/// 一条 SSE 流式事件帧。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StreamEvent {
    /// 事件名(如 "message" / "completed")。
    pub event: String,
    /// 事件负载(JSON 文本)。
    pub data: String,
}

/// transport 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(pub String);

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportError {}

/// 服务端消息处理器:远端 agent 端点收到 message/send 时的回调。
#[async_trait]
pub trait AgentHandler: Send + Sync {
    /// 处理收到的消息,返回回复。
    async fn handle_message(&self, message: AgentMessage) -> Result<AgentMessage, TransportError>;
}

/// transport Seam(Service Definition):agent 端点间消息传输。
#[async_trait]
pub trait AgentTransport: Seam {
    /// 本端点 agent 卡。
    fn local_card(&self) -> AgentCard;

    /// 拉取远端 agent 卡(JSON-RPC agent/getCard)。
    async fn fetch_card(&self, peer_url: &str) -> Result<AgentCard, TransportError>;

    /// 向远端发送消息(JSON-RPC message/send),返回回复。
    async fn send_message(
        &self,
        peer_url: &str,
        message: AgentMessage,
    ) -> Result<AgentMessage, TransportError>;

    /// 流式发送(SSE text/event-stream):返回解析出的帧序列,以 [DONE] 终止。
    async fn stream_send(
        &self,
        peer_url: &str,
        message: AgentMessage,
    ) -> Result<Vec<StreamEvent>, TransportError>;
}
