//! Profile 组合配置:bundle 顺序 + 插件清单(TOML)。

use std::path::Path;

use serde::Deserialize;

/// 组合配置:按顺序堆叠的 bundle 列表。
///
/// 对齐 DSH:profile 列出它堆叠的 bundle,bundle 列出其插件;
/// 展开后的插件顺序即挂载顺序(依赖拓扑仍由插件声明决定)。
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub bundles: Vec<Bundle>,
}

/// 一个 bundle:一组插件的分发单元。
#[derive(Debug, Clone, Deserialize)]
pub struct Bundle {
    pub id: String,
    #[serde(default)]
    pub plugins: Vec<String>,
    /// Optional plugin configuration kept as data for the host composer.
    #[serde(default)]
    pub config: Option<toml::Value>,
}

/// Profile 解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// 文件读取失败。
    Io(String),
    /// TOML 解析失败。
    Parse(String),
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "profile io error: {message}"),
            Self::Parse(message) => write!(f, "profile parse error: {message}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    /// 从 TOML 文本解析。
    pub fn from_toml(input: &str) -> Result<Self, ProfileError> {
        toml::from_str(input).map_err(|error| ProfileError::Parse(error.to_string()))
    }

    /// 从文件加载。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|error| ProfileError::Io(error.to_string()))?;
        Self::from_toml(&text)
    }

    /// 展开后的插件名列表(按 bundle 顺序,去重)。
    pub fn plugin_names(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for bundle in &self.bundles {
            for name in &bundle.plugins {
                if seen.insert(name.clone()) {
                    names.push(name.clone());
                }
            }
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profile_toml() {
        let toml = r#"name = "dev"

[[bundles]]
id = "mock"
plugins = ["ah-plugins-mock"]

[[bundles]]
id = "base"
plugins = ["ah-plugins-mock", "ah-plugins-otel"]
"#;
        let profile = Profile::from_toml(toml).expect("parse");
        assert_eq!(profile.name, "dev");
        assert_eq!(profile.bundles.len(), 2);
        assert!(profile.bundles[0].config.is_none());
        // 去重且保持顺序。
        assert_eq!(
            profile.plugin_names(),
            vec!["ah-plugins-mock", "ah-plugins-otel"]
        );
    }

    #[test]
    fn preserves_plugin_configuration_for_host_composer() {
        let profile = Profile::from_toml(
            r#"name = "prod"
            [[bundles]]
            id = "models"
            plugins = ["ah-plugins-model-backup"]
            [bundles.config]
            backup_providers = ["secondary"]
            retries_per_model = 1
            "#,
        )
        .expect("parse config");
        let config = profile.bundles[0].config.as_ref().unwrap();
        assert_eq!(
            config["backup_providers"].as_array().unwrap()[0].as_str(),
            Some("secondary")
        );
        assert_eq!(config["retries_per_model"].as_integer(), Some(1));
    }

    #[test]
    fn rejects_invalid_toml() {
        let error = Profile::from_toml("not [valid").expect_err("invalid");
        assert!(matches!(error, ProfileError::Parse(_)));
    }
}
