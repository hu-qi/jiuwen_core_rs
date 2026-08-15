//! oauth seam:设备码授权流(Device Authorization Grant)。
//!
//! 真实协议:start_device_flow 请求设备码(用户访问 verification_uri 输入
//! user_code),poll_token 按 interval 轮询换取 token;authorization_pending
//! 映射为 Pending,expired_token 映射为 Expired。

use async_trait::async_trait;

use crate::seam::Seam;

/// 设备码请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeviceAuthRequest {
    pub client_id: String,
    #[serde(default)]
    pub scope: Option<String>,
}

/// 设备码响应。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// token 响应。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// 轮询结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PollResult {
    /// 用户尚未授权,继续轮询。
    Pending,
    Token(TokenResponse),
    /// 设备码过期。
    Expired,
}

/// oauth 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthError(pub String);

impl core::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OAuthError {}

/// oauth Seam(Service Definition):设备码授权流。
#[async_trait]
pub trait OAuthClient: Seam {
    /// 发起设备码授权(真实 POST /device/authorization)。
    async fn start_device_flow(
        &self,
        issuer_url: &str,
        request: DeviceAuthRequest,
    ) -> Result<DeviceCode, OAuthError>;

    /// 轮询 token(真实 POST /token,form-encoded)。
    async fn poll_token(
        &self,
        issuer_url: &str,
        device_code: &str,
        interval: u64,
    ) -> Result<PollResult, OAuthError>;
}
