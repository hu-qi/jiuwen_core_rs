//! model-catalog seam:OpenAI 账号模型目录(对齐 extensions/external_provider/openai_auth/openai_account_models.py)。
//!
//! - 实时获取:GET {base_url}/models(Bearer token)→ 解析可见模型;
//! - 解析规则:entries 来自 `models`/`data`(list 或 dict);过滤 visibility ∈ {hide,hidden};
//!   id 取 slug/id/name/model 首个非空;按 (priority, id) 排序去重;
//! - 前向兼容扩展:模板模型存在时追加合成模型(gpt-5.5 ← gpt-5.4* 等);
//! - 兜底链:实时 → 本地缓存 JSON → 内置默认模型列表。
//!
//! 契约零实现:解析/缓存/HTTP 由插件提供(如 ah-plugins-model-catalog)。

use serde_json::Value;

use crate::seam::Seam;

/// 模型目录错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogError {
    pub message: String,
    pub status_code: Option<u16>,
}

impl ModelCatalogError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status_code: None,
        }
    }

    pub fn with_status(message: impl Into<String>, status_code: u16) -> Self {
        Self {
            message: message.into(),
            status_code: Some(status_code),
        }
    }
}

impl core::fmt::Display for ModelCatalogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.status_code {
            Some(code) => write!(f, "{} (status {code})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ModelCatalogError {}

use crate::llm::ModelProvider;
/// OpenAI 账号模型目录 Seam(Service Definition)。
use std::sync::Arc;

/// Named provider resolution seam used by fallback consumers.
pub trait ModelProviderCatalog: Seam {
    fn resolve(&self, name: &str) -> Result<Arc<dyn ModelProvider>, ModelCatalogError>;
    fn names(&self) -> Vec<String>;
}

pub trait ModelCatalog: Seam {
    /// 解析模型负载为有序模型 id 列表(过滤/排序/去重/前向兼容)。
    fn parse_model_ids(&self, payload: &Value) -> Vec<String>;

    /// 前向兼容扩展(模板存在时追加合成模型;保持顺序与去重)。
    fn add_forward_compat(&self, model_ids: Vec<String>) -> Vec<String>;

    /// 实时获取:GET {base_url}/models;返回 (payload, 有序模型 id)。
    fn fetch_models(
        &self,
        access_token: &str,
        base_url: &str,
    ) -> Result<(Value, Vec<String>), ModelCatalogError>;

    /// 完整列表:token → 实时 → 缓存 → 内置默认(对齐 Python 兜底链)。
    fn list_model_ids(
        &self,
        access_token: Option<&str>,
        base_url: &str,
        cache_path: &str,
        force_refresh: bool,
    ) -> Vec<String>;

    /// 读本地缓存中的模型 id 列表(无缓存返回空)。
    fn read_cache(&self, cache_path: &str) -> Vec<String>;

    /// 写本地缓存(payload + model_ids)。
    fn write_cache(
        &self,
        cache_path: &str,
        payload: &Value,
        model_ids: &[String],
    ) -> Result<(), ModelCatalogError>;
}
