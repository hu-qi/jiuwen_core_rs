//! web seam:HTTP 客户端(真实请求)。
//!
//! 对应 openjiuwen/harness 的 web 能力:真实 HTTP 请求(GET,可配超时),
//! 返回状态/响应头/正文。TLS(https)与流式响应由插件侧按特性扩展。

use crate::seam::Seam;

/// HTTP 获取请求。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WebFetchRequest {
    pub url: String,
    /// 超时毫秒;None 用实现默认。
    pub timeout_ms: Option<u64>,
}

/// HTTP 获取结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WebFetchResult {
    pub status: u16,
    /// 响应头(名称 → 值,按到达顺序)。
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub duration_ms: u64,
}

/// web 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebError(pub String);

impl core::fmt::Display for WebError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WebError {}

/// web Seam(Service Definition):真实 HTTP 客户端。
pub trait WebProvider: Seam {
    /// 发起 GET 请求并返回真实响应。
    fn fetch(&self, request: WebFetchRequest) -> Result<WebFetchResult, WebError>;
}
