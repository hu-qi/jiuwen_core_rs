//! # ah-plugins-transport
//!
//! 真实 A2A 风格 agent 传输(JSON-RPC 2.0 over HTTP):
//! - JsonRpcTransport:ureq 客户端,agent/getCard 与 message/send;
//! - AgentHttpServer:最小 HTTP/1.1 + JSON-RPC 服务端(真实 TCP 协议路径),
//!   AgentHandler 处理 message/send。
//!
//! 完整 A2A 规范(SSE/流式)留待后续,文档注明。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use ah_contracts::keys::TRANSPORT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::transport::{
    AgentCard, AgentHandler, AgentMessage, AgentTransport, StreamEvent, TransportError,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};
const JSONRPC_VERSION: &str = "2.0";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 JSON-RPC 客户端(ureq)。
pub struct JsonRpcTransport {
    card: AgentCard,
}

impl JsonRpcTransport {
    /// 以本端点 agent 卡创建。
    pub fn new(card: AgentCard) -> Self {
        Self { card }
    }

    /// 构造 JSON-RPC 2.0 请求并 POST,返回 result 或显式错误。
    fn call(&self, url: &str, method: &str, params: Value) -> Result<Value, TransportError> {
        self.call_with_version(url, method, params, JSONRPC_VERSION)
    }

    fn call_with_version(
        &self,
        url: &str,
        method: &str,
        params: Value,
        version: &str,
    ) -> Result<Value, TransportError> {
        let request_id = now_ms();
        let request = json!({
            "jsonrpc": version,
            "id": request_id,
            "method": method,
            "params": params,
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let payload = serde_json::to_string(&request).expect("serialize");
        let response = agent
            .post(url)
            .set("Content-Type", "application/json")
            .send_string(&payload)
            .map_err(|e| TransportError(format!("rpc call failed: {e}")))?;
        let body: Value = response
            .into_string()
            .map_err(|e| TransportError(format!("read rpc body failed: {e}")))?
            .parse()
            .map_err(|e| TransportError(format!("parse rpc body failed: {e}")))?;
        if body.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
            return Err(TransportError(
                "unsupported JSON-RPC version in response".to_string(),
            ));
        }
        if body.get("id") != Some(&json!(request_id)) {
            return Err(TransportError("JSON-RPC response id mismatch".to_string()));
        }
        if let Some(error) = body.get("error") {
            let message = error
                .as_object()
                .and_then(|o| o.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(TransportError(format!("rpc error: {message}")));
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| TransportError("rpc response missing result".to_string()))
    }
}

impl Seam for JsonRpcTransport {}

#[async_trait]
impl AgentTransport for JsonRpcTransport {
    fn local_card(&self) -> AgentCard {
        self.card.clone()
    }

    async fn fetch_card(&self, peer_url: &str) -> Result<AgentCard, TransportError> {
        let result = self.call(peer_url, "agent/getCard", json!({}))?;
        serde_json::from_value(result).map_err(|e| TransportError(format!("parse agent card: {e}")))
    }

    async fn send_message(
        &self,
        peer_url: &str,
        message: AgentMessage,
    ) -> Result<AgentMessage, TransportError> {
        let result = self.call(peer_url, "message/send", json!({ "message": message }))?;
        serde_json::from_value(result)
            .map_err(|e| TransportError(format!("parse reply message: {e}")))
    }

    async fn stream_send(
        &self,
        peer_url: &str,
        message: AgentMessage,
    ) -> Result<Vec<StreamEvent>, TransportError> {
        // SSE 请求:POST + Accept: text/event-stream,读流式响应并解析 data 帧。
        let request = json!({
            "jsonrpc": "2.0",
            "id": now_ms(),
            "method": "message/send",
            "params": { "message": message },
        });
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let payload = serde_json::to_string(&request).expect("serialize");
        let response = agent
            .post(peer_url)
            .set("Content-Type", "application/json")
            .set("Accept", "text/event-stream")
            .send_string(&payload)
            .map_err(|e| TransportError(format!("sse call failed: {e}")))?;
        let mut reader = response.into_reader();
        let mut text = String::new();
        use std::io::Read;
        reader
            .read_to_string(&mut text)
            .map_err(|e| TransportError(format!("read sse: {e}")))?;
        // 解析 data: 帧(SSE 规范:data: <json> 后接空行)。
        let mut events = Vec::new();
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                let data = data.trim();
                if data == "[DONE]" {
                    break;
                }
                if let Ok(frame) = serde_json::from_str::<Value>(data) {
                    events.push(StreamEvent {
                        event: frame
                            .get("event")
                            .and_then(Value::as_str)
                            .unwrap_or("data")
                            .to_string(),
                        data: frame
                            .get("data")
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| data.to_string()),
                    });
                }
            }
        }
        Ok(events)
    }
}

// ---------------- 服务端 ----------------

/// 最小 HTTP/1.1 + JSON-RPC 服务端(真实 TCP 协议路径)。
pub struct AgentHttpServer {
    listener: TcpListener,
    card: AgentCard,
    handler: Arc<dyn AgentHandler>,
}

impl AgentHttpServer {
    /// 绑定 127.0.0.1 随机端口并后台接受连接。
    pub fn serve(card: AgentCard, handler: Arc<dyn AgentHandler>) -> Result<Self, TransportError> {
        let listener =
            TcpListener::bind("127.0.0.1:0").map_err(|e| TransportError(format!("bind: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| TransportError(format!("addr: {e}")))?;
        let url = format!("http://{addr}/");
        let mut card = card;
        card.url = url;
        let server = Self {
            listener,
            card,
            handler,
        };
        let handle = server
            .listener
            .try_clone()
            .map_err(|e| TransportError(e.to_string()))?;
        let card_clone = server.card.clone();
        let handler_clone = server.handler.clone();
        std::thread::spawn(move || {
            for stream in handle.incoming().flatten() {
                let card = card_clone.clone();
                let handler = handler_clone.clone();
                std::thread::spawn(move || {
                    let _ = handle_connection(stream, card, handler);
                });
            }
        });
        Ok(server)
    }

    /// 端点 URL。
    pub fn url(&self) -> String {
        self.card.url.clone()
    }
}

/// 处理单个 HTTP 连接:解析请求行/头/体,分发 JSON-RPC,回写响应。
fn handle_connection(
    mut stream: TcpStream,
    card: AgentCard,
    handler: Arc<dyn AgentHandler>,
) -> Result<(), TransportError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    // 阶段 1:读至请求头结束(CRLF 分隔符出现为止)。
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            break;
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| TransportError(format!("read: {e}")))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    // 阶段 2:继续读到 Content-Length 声明的请求体完整到达(避免分包竞态)。
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(buf.len());
    let content_length = content_length_of(&buf[..header_end]);
    while buf.len() < header_end + content_length {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| TransportError(format!("read body: {e}")))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body_bytes = &buf[header_end..];
    let body: String = body_bytes
        .iter()
        .take(content_length)
        .map(|&b| b as char)
        .collect();

    // SSE 流式:请求头 Accept: text/event-stream 时走流式响应。
    let headers_text = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let wants_stream = headers_text
        .to_lowercase()
        .contains("accept: text/event-stream");
    if wants_stream {
        return stream_response(&mut stream, &body, &card, &handler);
    }

    let response_body = dispatch(&body, &card, &handler);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|e| TransportError(format!("write: {e}")))
}

/// SSE 流式响应:逐个写入 data 帧,以 [DONE] 终止(真实 text/event-stream)。
fn stream_response(
    stream: &mut TcpStream,
    body: &str,
    card: &AgentCard,
    _handler: &Arc<dyn AgentHandler>,
) -> Result<(), TransportError> {
    use std::io::Write;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n",
        )
        .map_err(|e| TransportError(format!("sse headers: {e}")))?;
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let message: AgentMessage = serde_json::from_value(
        parsed
            .get("params")
            .and_then(|p| p.get("message"))
            .cloned()
            .unwrap_or(Value::Null),
    )
    .unwrap_or(AgentMessage {
        id: "unknown".to_string(),
        role: "user".to_string(),
        content: String::new(),
        kind: "text".to_string(),
    });
    // 进度叙述:phase_started -> agent_started -> message -> completed。
    let frames = [
        json!({ "event": "phase_started", "data": json!({ "name": "A2A stream" }) }),
        json!({ "event": "agent_started", "data": json!({ "agent": message.role }) }),
        json!({ "event": "message", "data": json!({ "content": format!("card: {}", card.name) }) }),
        json!({ "event": "completed", "data": json!({ "ok": true }) }),
    ];
    for frame in frames {
        let line = format!("data: {}\r\n\r\n", serde_json::to_string(&frame).unwrap());
        stream
            .write_all(line.as_bytes())
            .map_err(|e| TransportError(format!("sse frame: {e}")))?;
        stream
            .flush()
            .map_err(|e| TransportError(format!("sse flush: {e}")))?;
    }
    stream
        .write_all(b"data: [DONE]\r\n\r\n")
        .map_err(|e| TransportError(format!("sse done: {e}")))
}

/// 从请求头字节中解析 Content-Length。
fn content_length_of(headers: &[u8]) -> usize {
    let text = String::from_utf8_lossy(headers);
    let mut content_length = 0usize;
    for line in text.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    content_length
}

/// 分发 JSON-RPC 方法并返回响应 JSON。
fn dispatch(body: &str, card: &AgentCard, handler: &Arc<dyn AgentHandler>) -> String {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(e) => {
            return json!({
                "jsonrpc": "2.0", "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            })
            .to_string();
        }
    };
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    if parsed.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "error": { "code": -32600, "message": "unsupported JSON-RPC version" }
        })
        .to_string();
    }
    let method = parsed
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "agent/getCard" => serde_json::to_value(card).unwrap_or(Value::Null),
        "message/send" => {
            let message: Result<AgentMessage, _> = serde_json::from_value(
                parsed
                    .get("params")
                    .and_then(|p| p.get("message"))
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            match message {
                Ok(message) => {
                    let reply = handler.handle_message(message);
                    // 同步桥:handler 为 async,这里用 tokio 运行时驱动。
                    match tokio::runtime::Runtime::new()
                        .map(|rt| rt.block_on(reply))
                        .map_err(|e| TransportError(format!("runtime: {e}")))
                    {
                        Ok(Ok(reply)) => serde_json::to_value(reply).unwrap_or(Value::Null),
                        Ok(Err(e)) => {
                            return json!({
                                "jsonrpc": "2.0", "id": id,
                                "error": { "code": -32000, "message": e.0 }
                            })
                            .to_string();
                        }
                        Err(e) => {
                            return json!({
                                "jsonrpc": "2.0", "id": id,
                                "error": { "code": -32000, "message": e.0 }
                            })
                            .to_string();
                        }
                    }
                }
                Err(e) => {
                    return json!({
                        "jsonrpc": "2.0", "id": id,
                        "error": { "code": -32602, "message": format!("invalid params: {e}") }
                    })
                    .to_string();
                }
            }
        }
        other => {
            return json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": format!("method not found: {other}") }
            })
            .to_string();
        }
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// transport 插件:提供本端点 agent 卡与客户端。
pub struct TransportPlugin {
    card: AgentCard,
}

impl TransportPlugin {
    /// 以本端点 agent 卡创建。
    pub fn new(card: AgentCard) -> Self {
        Self { card }
    }
}

impl Plugin for TransportPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-transport"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TRANSPORT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let transport: Arc<dyn AgentTransport> = Arc::new(JsonRpcTransport::new(self.card.clone()));
        Ok(vec![ctx.register(TRANSPORT, transport)])
    }
}

/// 测试用回声处理器(真实消息往返;测试专用桩)。
pub struct EchoHandler;

#[async_trait]
impl AgentHandler for EchoHandler {
    async fn handle_message(&self, message: AgentMessage) -> Result<AgentMessage, TransportError> {
        Ok(AgentMessage {
            id: format!("reply-{}", message.id),
            role: "assistant".to_string(),
            content: format!("echo: {}", message.content),
            kind: "text".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TRANSPORT;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn card() -> AgentCard {
        AgentCard {
            name: "test-agent".to_string(),
            description: "a test agent".to_string(),
            url: "http://unset/".to_string(),
            skills: vec!["list".to_string(), "search".to_string()],
        }
    }

    fn build_ctx(card: AgentCard) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(TransportPlugin::new(card))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn fetch_card_and_send_message_over_real_http() {
        let server = AgentHttpServer::serve(card(), StdArc::new(EchoHandler)).expect("serve");
        let url = server.url();

        let (ctx, effects) = build_ctx(card());
        let transport = ctx
            .service::<dyn AgentTransport>(&TRANSPORT)
            .expect("transport");

        // 拉取远端 agent 卡(真实 JSON-RPC agent/getCard)。
        let remote_card = transport.fetch_card(&url).await.expect("fetch card");
        assert_eq!(remote_card.name, "test-agent");
        assert!(remote_card.skills.contains(&"search".to_string()));
        assert_eq!(remote_card.url, url, "server injects its own url");

        // 发送消息(真实 JSON-RPC message/send),得到回声回复。
        let reply = transport
            .send_message(
                &url,
                AgentMessage {
                    id: "m1".to_string(),
                    role: "user".to_string(),
                    content: "hello peer".to_string(),
                    kind: "text".to_string(),
                },
            )
            .await
            .expect("send");
        assert_eq!(reply.content, "echo: hello peer");
        assert_eq!(reply.role, "assistant");

        drop(effects);
    }

    #[tokio::test]
    async fn unknown_method_errors_explicitly() {
        let server = AgentHttpServer::serve(card(), StdArc::new(EchoHandler)).expect("serve");
        let (ctx, effects) = build_ctx(card());
        let transport = ctx
            .service::<dyn AgentTransport>(&TRANSPORT)
            .expect("transport");
        // seam 暴露的方法仅 agent/getCard 与 message/send;未知方法走原始 RPC 验证。
        assert_eq!(transport.local_card().name, "test-agent");

        let raw = JsonRpcTransport::new(card());
        let err = raw
            .call(&server.url(), "no/suchMethod", json!({}))
            .expect_err("unknown method");
        assert!(err.0.contains("method not found"));

        drop(effects);
    }

    #[test]
    fn rejects_unsupported_jsonrpc_protocol_version() {
        let server = AgentHttpServer::serve(card(), StdArc::new(EchoHandler)).expect("serve");
        let raw = JsonRpcTransport::new(card());
        let error = raw
            .call_with_version(&server.url(), "agent/getCard", json!({}), "1.0")
            .expect_err("unsupported JSON-RPC version");
        assert!(error.0.contains("unsupported JSON-RPC version"));
    }

    #[test]
    fn rejects_unsupported_jsonrpc_response_version() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind response fixture");
        let address = listener.local_addr().expect("response fixture address");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept response fixture");
            let mut request = [0_u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut request);
            let body = r#"{"jsonrpc":"1.0","id":0,"result":{}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            std::io::Write::write_all(&mut stream, response.as_bytes()).expect("write response");
        });
        let raw = JsonRpcTransport::new(card());
        let error = raw
            .call(&format!("http://{address}"), "agent/getCard", json!({}))
            .expect_err("unsupported response version");
        assert!(error.0.contains("unsupported JSON-RPC version in response"));
    }

    #[test]
    fn raw_tcp_client_receives_http_response() {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let server = AgentHttpServer::serve(card(), StdArc::new(EchoHandler)).expect("serve");
        let url = server.url();
        let addr = url.trim_start_matches("http://").trim_end_matches('/');
        let mut stream = TcpStream::connect(addr).expect("connect");
        let request = format!(
            "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agent/getCard\",\"params\":{}}".len(),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agent/getCard\",\"params\":{}}"
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).expect("read");
        let text = String::from_utf8_lossy(&buf).into_owned();
        println!("SERVER RESPONSE: {text}");
        assert!(text.contains("HTTP/1.1 200"), "http status line: {text}");
    }

    #[tokio::test]
    async fn stream_send_receives_sse_frames_in_order() {
        let server = AgentHttpServer::serve(card(), StdArc::new(EchoHandler)).expect("serve");
        let url = server.url();
        let (ctx, effects) = build_ctx(card());
        let transport = ctx
            .service::<dyn AgentTransport>(&TRANSPORT)
            .expect("transport");

        let events = transport
            .stream_send(
                &url,
                AgentMessage {
                    id: "s1".to_string(),
                    role: "user".to_string(),
                    content: "stream me".to_string(),
                    kind: "text".to_string(),
                },
            )
            .await
            .expect("stream");
        assert_eq!(events.len(), 4, "phase/agent/message/completed frames");
        assert_eq!(events[0].event, "phase_started");
        assert_eq!(events[1].event, "agent_started");
        assert_eq!(events[2].event, "message");
        assert_eq!(events[3].event, "completed");
        assert!(events[2].data.contains("test-agent"));

        drop(effects);
    }
}
