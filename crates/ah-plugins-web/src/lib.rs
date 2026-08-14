//! # ah-plugins-web
//!
//! 真实 HTTP 客户端(对应 openjiuwen/harness 的 web 能力):ureq 发 GET,
//! 可配超时;返回真实状态码/响应头/正文。TLS(https)需启用 ureq tls
//! 特性,当前默认关闭(plain http),文档注明。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ah_contracts::keys::{TOOLS, WEB};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_contracts::web::{WebError, WebFetchRequest, WebFetchResult, WebProvider};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 真实 HTTP 客户端(ureq 后端)。
pub struct UreqWebProvider;

impl Seam for UreqWebProvider {}

impl WebProvider for UreqWebProvider {
    fn fetch(&self, request: WebFetchRequest) -> Result<WebFetchResult, WebError> {
        let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(10_000));
        let agent = ureq::AgentBuilder::new().timeout(timeout).build();
        let started = now_ms();
        let response = agent
            .get(&request.url)
            .call()
            .map_err(|e| WebError(format!("request failed: {e}")))?;
        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers_names()
            .into_iter()
            .filter_map(|name| {
                response
                    .header(&name)
                    .map(|value| (name, value.to_string()))
            })
            .collect();
        let body = response
            .into_string()
            .map_err(|e| WebError(format!("read body failed: {e}")))?;
        Ok(WebFetchResult {
            status,
            headers,
            body,
            duration_ms: now_ms().saturating_sub(started),
        })
    }
}

/// web_fetch 工具:agent 经工具管线真实发起 HTTP GET。
pub struct WebFetchTool {
    web: std::sync::Arc<dyn WebProvider>,
}

impl WebFetchTool {
    pub fn new(web: std::sync::Arc<dyn WebProvider>) -> Self {
        Self { web }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "fetch a URL over HTTP; arguments: {url, timeout_ms?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "timeout_ms": { "type": "integer" },
            },
            "required": ["url"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let url = arguments
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field url".to_string()))?;
        let timeout_ms = arguments.get("timeout_ms").and_then(Value::as_u64);
        let result = self
            .web
            .fetch(WebFetchRequest {
                url: url.to_string(),
                timeout_ms,
            })
            .map_err(|e| ToolError(format!("fetch failed: {e}")))?;
        Ok(json!({
            "status": result.status,
            "body": result.body.chars().take(1000).collect::<String>(),
        }))
    }
}

/// web 插件:提供真实 HTTP 客户端。
pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-web"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![WEB]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: std::sync::Arc<dyn WebProvider> = std::sync::Arc::new(UreqWebProvider);
        let mut effects = vec![ctx.register(WEB, provider.clone())];
        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(std::sync::Arc::new(WebFetchTool::new(provider))));
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::WEB;
    use ah_hub::plugin::DynPlugin;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc as StdArc;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(WebPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    /// 真实本地 HTTP 服务器(真实 TCP 协议路径,非工具桩)。
    fn spawn_local_server(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (url, handle)
    }

    #[test]
    fn fetches_real_local_server() {
        let (url, server) = spawn_local_server("hello from local server");
        let (ctx, effects) = build_ctx();
        let web = ctx.service::<dyn WebProvider>(&WEB).expect("web");

        let result = web
            .fetch(WebFetchRequest {
                url,
                timeout_ms: Some(5000),
            })
            .expect("fetch");
        assert_eq!(result.status, 200);
        assert_eq!(result.body, "hello from local server");
        assert!(
            result
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type")),
            "response headers captured"
        );

        server.join().expect("server");
        drop(effects);
    }

    #[tokio::test]
    async fn web_fetch_tool_invokes_real_http() {
        use ah_contracts::keys::TOOLS;
        use ah_contracts::tools::ToolRegistry;
        use ah_hub::plugin::DynPlugin;
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::Arc as StdArc;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}/");
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "tool works";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(WebPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
        let result = registry
            .invoke("web_fetch", json!({ "url": url }))
            .await
            .expect("web_fetch");
        assert_eq!(result["status"], 200);
        assert_eq!(result["body"], "tool works");

        handle.join().expect("server");
        drop(effects);
    }

    #[test]
    fn unreachable_host_errors_explicitly() {
        let (ctx, effects) = build_ctx();
        let web = ctx.service::<dyn WebProvider>(&WEB).expect("web");

        // 绑定后立即释放端口 → 连接必失败(显式报错,不静默)。
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let err = web
            .fetch(WebFetchRequest {
                url: format!("http://127.0.0.1:{port}/"),
                timeout_ms: Some(2000),
            })
            .expect_err("connection refused must error");
        assert!(err.0.contains("request failed") || err.0.contains("refused"));

        drop(effects);
    }
}
