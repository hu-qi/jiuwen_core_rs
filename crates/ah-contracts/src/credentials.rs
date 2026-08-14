//! credentials seam:凭据引用(credential reference)。
//!
//! 凭据是敏感信息,不应直接写入 profile/代码。本 seam 提供按名解析凭据的
//! 统一接口,消费方(如 ah-plugins-openai)通过 `credentials` 服务键解析
//! `CredentialProvider`,按稳定名称(如 `openai.api_key`)取用。
//!
//! 契约零实现:存储与解析(环境变量 / 密钥管理后端)由插件提供。

use crate::seam::Seam;

/// 一条已解析的凭据。
///
/// - `name`:稳定凭据名(如 `openai.api_key`、`openai.base_url`);
/// - `value`:凭据值(明文;消费方负责妥善处理);
/// - `source`:来源描述(如 `env:OPENAI_API_KEY`),用于诊断与审计。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Credential {
    /// 稳定凭据名。
    pub name: String,
    /// 凭据值。
    pub value: String,
    /// 来源描述(如 `env:OPENAI_API_KEY`)。
    pub source: String,
}

/// 凭据错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialError(pub String);

impl core::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CredentialError {}

/// credentials Seam(Service Definition):按名解析/管理凭据。
///
/// 消费方只依赖本 trait,不 import 具体实现(环境变量 / 密钥管理后端)。
///
/// 语义约定:
/// - `get`:解析一个凭据;未配置或不可用返回 None;
/// - `set` / `remove`:仅供测试或显式注入。只读后端(如环境变量)
///   必须返回显式错误,禁止静默 no-op;
/// - `list`:返回当前可用的全部凭据(按名称排序)。
pub trait CredentialProvider: Seam {
    /// 按名解析一个凭据;未配置或不可用时返回 None。
    fn get(&self, name: &str) -> Option<Credential>;

    /// 设置一个凭据(仅供测试/显式注入)。
    ///
    /// `source` 由调用方标注来源(如 `test`、`injected`)。
    /// 只读后端必须返回显式错误。
    fn set(&self, name: &str, value: &str, source: &str) -> Result<Credential, CredentialError>;

    /// 当前可用的全部凭据(实现方保证确定性顺序)。
    fn list(&self) -> Vec<Credential>;

    /// 删除一个凭据;只读后端必须返回显式错误。
    fn remove(&self, name: &str) -> Result<(), CredentialError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_serde_roundtrip() {
        let credential = Credential {
            name: "openai.api_key".to_string(),
            value: "sk-test".to_string(),
            source: "env:OPENAI_API_KEY".to_string(),
        };
        let wire = serde_json::to_value(&credential).expect("serialize");
        let back: Credential = serde_json::from_value(wire).expect("deserialize");
        assert_eq!(back, credential);
        assert_eq!(back.name, "openai.api_key");
        assert_eq!(back.source, "env:OPENAI_API_KEY");
    }

    #[test]
    fn credential_error_displays_message() {
        let error = CredentialError("env provider is read-only".to_string());
        assert_eq!(error.to_string(), "env provider is read-only");
        // 必须实现 std::error::Error(可上抛)。
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert!(!boxed.to_string().is_empty());
    }

    #[test]
    fn credential_fields_are_plain_data() {
        // 契约层只声明纯类型:字段公开、可 Clone/PartialEq。
        let a = Credential {
            name: "k".into(),
            value: "v".into(),
            source: "s".into(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
