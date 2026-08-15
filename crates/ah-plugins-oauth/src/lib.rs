//! # ah-plugins-oauth
//!
//! 真实设备码 OAuth 客户端(Device Authorization Grant,OpenAI 等 provider 使用):
//! start_device_flow POST /device/authorization;poll_token POST /token
//! (application/x-www-form-urlencoded),把 authorization_pending / expired_token
//! 映射为 Pending / Expired。全程真实 HTTP。

use std::sync::Arc;
use std::time::Duration;

use ah_contracts::keys::OAUTH;
use ah_contracts::oauth::{
    DeviceAuthRequest, DeviceCode, OAuthClient, OAuthError, PollResult, TokenResponse,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::Value;

/// 真实设备码 OAuth 客户端。
pub struct DeviceCodeOAuthClient {
    agent: ureq::Agent,
}

impl DeviceCodeOAuthClient {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }
}

impl Default for DeviceCodeOAuthClient {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for DeviceCodeOAuthClient {}

#[async_trait]
impl OAuthClient for DeviceCodeOAuthClient {
    async fn start_device_flow(
        &self,
        issuer_url: &str,
        request: DeviceAuthRequest,
    ) -> Result<DeviceCode, OAuthError> {
        let agent = self.agent.clone();
        let issuer_url = issuer_url.to_string();
        let client_id = request.client_id.clone();
        let scope = request.scope.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut form: Vec<(&str, &str)> = vec![("client_id", &client_id)];
            if let Some(scope) = scope.as_deref() {
                form.push(("scope", scope));
            }
            agent
                .post(&format!("{issuer_url}/device/authorization"))
                .send_form(&form)
                .map_err(|e| OAuthError(format!("device auth request failed: {e}")))?
                .into_string()
                .map_err(|e| OAuthError(format!("read device auth response failed: {e}")))
        })
        .await
        .map_err(|e| OAuthError(format!("oauth task failed: {e}")))??;
        let value: Value = serde_json::from_str(&result)
            .map_err(|e| OAuthError(format!("parse device code: {e}")))?;
        Ok(DeviceCode {
            device_code: value
                .get("device_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            user_code: value
                .get("user_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            verification_uri: value
                .get("verification_uri")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            expires_in: value.get("expires_in").and_then(Value::as_u64).unwrap_or(0),
            interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
        })
    }

    async fn poll_token(
        &self,
        issuer_url: &str,
        device_code: &str,
        interval: u64,
    ) -> Result<PollResult, OAuthError> {
        // 按 RFC 8628:轮询间至少等待 interval 秒。
        if interval > 0 {
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
        let agent = self.agent.clone();
        let issuer_url = issuer_url.to_string();
        let device_code = device_code.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let grant = "urn:ietf:params:oauth:grant-type:device_code";
            let form: [(&str, &str); 2] = [("grant_type", grant), ("device_code", &device_code)];
            agent
                .post(&format!("{issuer_url}/token"))
                .send_form(&form)
                .map_err(|e| OAuthError(format!("token request failed: {e}")))?
                .into_string()
                .map_err(|e| OAuthError(format!("read token response failed: {e}")))
        })
        .await
        .map_err(|e| OAuthError(format!("oauth task failed: {e}")))??;
        let value: Value = serde_json::from_str(&result)
            .map_err(|e| OAuthError(format!("parse token response: {e}")))?;
        // 错误码映射。
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return match error {
                "authorization_pending" => Ok(PollResult::Pending),
                "expired_token" => Ok(PollResult::Expired),
                other => Err(OAuthError(format!("oauth error: {other}"))),
            };
        }
        Ok(PollResult::Token(TokenResponse {
            access_token: value
                .get("access_token")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            token_type: value
                .get("token_type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            expires_in: value.get("expires_in").and_then(Value::as_u64),
        }))
    }
}

/// oauth 插件:提供设备码授权客户端。
pub struct OAuthPlugin;

impl Plugin for OAuthPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-oauth"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![OAUTH]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let client: Arc<dyn OAuthClient> = Arc::new(DeviceCodeOAuthClient::new());
        Ok(vec![ctx.register(OAUTH, client)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::OAUTH;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc as StdArc;

    /// 真实本地 OAuth 授权服务器(模拟 issuer 的 device/authorization 与 token)。
    fn spawn_issuer(pending_first: bool) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let url = format!("http://{addr}");
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                handle_issuer_conn(stream, pending_first);
            }
        });
        (url, handle)
    }

    fn handle_issuer_conn(mut stream: std::net::TcpStream, pending_first: bool) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        // 阶段 1:读至请求头结束。
        loop {
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        // 阶段 2:续读至 Content-Length 声明字节完整到达(避免分包竞态)。
        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(buf.len());
        let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let mut content_length = 0usize;
        for line in headers.lines().skip(1) {
            if let Some((name, value)) = line.split_once(':')
                && name.trim().eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        while buf.len() < header_end + content_length {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let text = String::from_utf8_lossy(&buf).into_owned();
        let body = match text.find("\r\n\r\n") {
            Some(i) => text[i + 4..].to_string(),
            None => String::new(),
        };
        let is_token = text.to_lowercase().contains("/token");
        let payload = if is_token {
            if pending_first {
                json!({ "error": "authorization_pending" })
            } else {
                json!({
                    "access_token": "tok-123",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                })
            }
        } else {
            json!({
                "device_code": "dev-1",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://example.com/device",
                "expires_in": 600,
                "interval": 1,
            })
        };
        let body_str = payload.to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body_str.len(),
            body_str
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = &body;
    }

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![StdArc::new(OAuthPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn device_flow_start_and_poll_until_token() {
        let (issuer_url, server) = spawn_issuer(false);
        let (ctx, effects) = build_ctx();
        let oauth = ctx.service::<dyn OAuthClient>(&OAUTH).expect("oauth");

        let code = oauth
            .start_device_flow(
                &issuer_url,
                DeviceAuthRequest {
                    client_id: "test-client".to_string(),
                    scope: Some("model.read".to_string()),
                },
            )
            .await
            .expect("start");
        assert_eq!(code.device_code, "dev-1");
        assert_eq!(code.user_code, "ABCD-EFGH");
        assert!(code.verification_uri.contains("example.com"));
        assert_eq!(code.interval, 1);

        let poll = oauth
            .poll_token(&issuer_url, &code.device_code, 0)
            .await
            .expect("poll");
        match poll {
            PollResult::Token(token) => {
                assert_eq!(token.access_token, "tok-123");
                assert_eq!(token.token_type, "Bearer");
            }
            other => panic!("expected token, got {other:?}"),
        }

        drop(effects);
        let _ = server;
    }

    #[tokio::test]
    async fn pending_maps_to_poll_result() {
        let (issuer_url, server) = spawn_issuer(true);
        let (ctx, effects) = build_ctx();
        let oauth = ctx.service::<dyn OAuthClient>(&OAUTH).expect("oauth");
        let poll = oauth
            .poll_token(&issuer_url, "dev-1", 0)
            .await
            .expect("poll");
        assert_eq!(poll, PollResult::Pending);
        drop(effects);
        let _ = server;
    }
}
