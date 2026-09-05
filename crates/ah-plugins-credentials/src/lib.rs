//! # ah-plugins-credentials
//!
//! 真实 credentials seam:环境变量后端的凭据 provider。
//!
//! - get 真实读取 std::env(按可配置的「凭据名 → 环境变量名」映射,
//!   如 openai.api_key → OPENAI_API_KEY、openai.base_url → OPENAI_BASE_URL);
//! - list 返回已配置映射中存在(环境变量已设置)的项;
//! - set / remove 对 env provider 显式报错(环境变量只读,
//!   禁止静默 no-op / fallback);如需要可写后端,由其它插件提供。
//!
//! 默认映射(可用 CredentialsPlugin::new / EnvCredentialProvider::new
//! 注入自定义映射):
//!
//! | 凭据名 | 环境变量 |
//! | --- | --- |
//! | openai.api_key | OPENAI_API_KEY |
//! | openai.base_url | OPENAI_BASE_URL |
//! | openai.model | OPENAI_MODEL |

use std::collections::HashMap;
use std::sync::Arc;

use ah_contracts::credentials::{Credential, CredentialError, CredentialProvider};
use ah_contracts::keys::CREDENTIALS;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 默认映射:凭据名 → 环境变量名。
fn default_mapping() -> HashMap<String, String> {
    let mut mapping = HashMap::new();
    mapping.insert("openai.api_key".to_string(), "OPENAI_API_KEY".to_string());
    mapping.insert("openai.base_url".to_string(), "OPENAI_BASE_URL".to_string());
    mapping.insert("openai.model".to_string(), "OPENAI_MODEL".to_string());
    mapping
}

/// 环境变量后端的凭据 provider。
///
/// 映射规则可配置:构造时传入「凭据名 → 环境变量名」映射;
/// 未在映射中的凭据名一律解析为 None(不 fallback 到任何猜测规则)。
pub struct EnvCredentialProvider {
    mapping: HashMap<String, String>,
}

impl EnvCredentialProvider {
    /// 以自定义映射构建(凭据名 → 环境变量名)。
    pub fn new(mapping: HashMap<String, String>) -> Self {
        Self { mapping }
    }

    /// 默认映射(openai.api_key → OPENAI_API_KEY 等)。
    pub fn default_mapping() -> HashMap<String, String> {
        default_mapping()
    }

    /// 当前映射(凭据名 → 环境变量名)。
    pub fn mapping(&self) -> &HashMap<String, String> {
        &self.mapping
    }
}

impl Default for EnvCredentialProvider {
    fn default() -> Self {
        Self::new(default_mapping())
    }
}

impl Seam for EnvCredentialProvider {}

impl CredentialProvider for EnvCredentialProvider {
    fn get(&self, name: &str) -> Option<Credential> {
        let env_var = self.mapping.get(name)?;
        let value = std::env::var(env_var).ok()?;
        if value.trim().is_empty() {
            return None;
        }
        Some(Credential {
            name: name.to_string(),
            value,
            source: format!("env:{env_var}"),
        })
    }

    fn set(&self, name: &str, _value: &str, _source: &str) -> Result<Credential, CredentialError> {
        // env 不可写:显式报错,禁止静默 no-op(契约语义要求)。
        Err(CredentialError(format!(
            "cannot set credential {name}: environment variables are read-only (env provider)"
        )))
    }

    fn list(&self) -> Vec<Credential> {
        // 已配置映射中存在(环境变量已设置)的项,按凭据名排序保证确定性。
        let mut names: Vec<&String> = self.mapping.keys().collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| self.get(name))
            .collect()
    }

    fn remove(&self, name: &str) -> Result<(), CredentialError> {
        // env 不可写:显式报错,禁止静默 no-op(契约语义要求)。
        Err(CredentialError(format!(
            "cannot remove credential {name}: environment variables are read-only (env provider)"
        )))
    }
}

/// credentials 插件:注册 credentials seam。
///
/// 无目录参数;映射可用 CredentialsPlugin::new 注入(默认见模块文档)。
pub struct CredentialsPlugin {
    mapping: HashMap<String, String>,
}

impl CredentialsPlugin {
    /// 以自定义映射构建(凭据名 → 环境变量名)。
    pub fn new(mapping: HashMap<String, String>) -> Self {
        Self { mapping }
    }

    /// 映射(凭据名 → 环境变量名)。
    pub fn mapping(&self) -> &HashMap<String, String> {
        &self.mapping
    }
}

impl Default for CredentialsPlugin {
    fn default() -> Self {
        Self {
            mapping: default_mapping(),
        }
    }
}

impl Plugin for CredentialsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-credentials"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CREDENTIALS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn CredentialProvider> =
            Arc::new(EnvCredentialProvider::new(self.mapping.clone()));
        Ok(vec![ctx.register(CREDENTIALS, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Mutex;

    /// 串行化环境变量修改(edition 2024 下 set_var 为 unsafe;
    /// 全部 env 测试经本锁串行,避免并行测试互相污染)。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 在设置的环境变量下执行 f,结束后恢复原值(panic 也恢复)。
    fn with_env_vars(vars: &[(&str, &str)], f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(name, _)| ((*name).to_string(), std::env::var(name).ok()))
            .collect();
        for (name, value) in vars {
            // SAFETY:测试内唯一修改者(ENV_LOCK 串行);恢复在 f 之后执行。
            unsafe { std::env::set_var(name, value) };
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        for (name, previous) in saved {
            match previous {
                Some(value) => unsafe { std::env::set_var(&name, &value) },
                None => unsafe { std::env::remove_var(&name) },
            }
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    /// 单条目测试映射:test.api_key → 给定 env 名。
    fn single_mapping(env_var: &str) -> HashMap<String, String> {
        let mut mapping = HashMap::new();
        mapping.insert("test.api_key".to_string(), env_var.to_string());
        mapping
    }

    #[test]
    fn get_reads_real_environment_variable() {
        let env_var = "AH_CRED_TEST_GET_KEY";
        with_env_vars(&[(env_var, "sk-real-secret")], || {
            let provider = EnvCredentialProvider::new(single_mapping(env_var));
            let credential = provider.get("test.api_key").expect("credential present");
            assert_eq!(credential.name, "test.api_key");
            assert_eq!(credential.value, "sk-real-secret");
            assert_eq!(credential.source, format!("env:{env_var}"));
        });
        // 清理后(环境变量已恢复为未设置)不再解析到该凭据。
        with_env_vars(&[], || {
            let provider = EnvCredentialProvider::new(single_mapping(env_var));
            assert!(provider.get("test.api_key").is_none());
        });
    }

    #[test]
    fn get_unknown_name_returns_none() {
        let provider = EnvCredentialProvider::default();
        // 未映射的凭据名:不猜测、不 fallback。
        assert!(provider.get("unknown.secret").is_none());
    }

    #[test]
    fn get_mapped_but_unset_env_returns_none() {
        let env_var = "AH_CRED_TEST_UNSET_VAR";
        with_env_vars(&[], || {
            let provider = EnvCredentialProvider::new(single_mapping(env_var));
            assert!(provider.get("test.api_key").is_none());
        });
    }
    #[test]
    fn get_empty_env_returns_none() {
        let env_var = "AH_CRED_TEST_EMPTY_VAR";
        with_env_vars(&[(env_var, "   ")], || {
            let provider = EnvCredentialProvider::new(single_mapping(env_var));
            assert!(provider.get("test.api_key").is_none());
        });
    }

    #[test]
    fn set_returns_explicit_read_only_error() {
        let provider = EnvCredentialProvider::default();
        let error = provider
            .set("openai.api_key", "sk-x", "test")
            .expect_err("env is read-only");
        assert!(
            error.to_string().contains("read-only"),
            "error should explain env read-only, got: {error}"
        );
    }

    #[test]
    fn remove_returns_explicit_read_only_error() {
        let provider = EnvCredentialProvider::default();
        let error = provider
            .remove("openai.api_key")
            .expect_err("env is read-only");
        assert!(error.to_string().contains("read-only"));
    }

    #[test]
    fn list_returns_only_present_env_vars_sorted() {
        let env_a = "AH_CRED_TEST_LIST_A";
        let env_b = "AH_CRED_TEST_LIST_B";
        let mut mapping = HashMap::new();
        mapping.insert("z.present".to_string(), env_a.to_string());
        mapping.insert("a.present".to_string(), env_b.to_string());
        mapping.insert(
            "missing.one".to_string(),
            "AH_CRED_TEST_LIST_MISSING".to_string(),
        );
        with_env_vars(&[(env_a, "va"), (env_b, "vb")], || {
            let provider = EnvCredentialProvider::new(mapping);
            let listed = provider.list();
            // 只有已设置环境变量的映射项出现,且按凭据名排序。
            let names: Vec<&str> = listed.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, vec!["a.present", "z.present"]);
            assert_eq!(listed[0].value, "vb");
            assert_eq!(listed[1].value, "va");
        });
    }

    #[test]
    fn default_mapping_covers_openai_credentials() {
        let mapping = EnvCredentialProvider::default_mapping();
        assert_eq!(
            mapping.get("openai.api_key").map(String::as_str),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(
            mapping.get("openai.base_url").map(String::as_str),
            Some("OPENAI_BASE_URL")
        );
        assert_eq!(
            mapping.get("openai.model").map(String::as_str),
            Some("OPENAI_MODEL")
        );
    }

    #[test]
    fn plugin_registers_credentials_seam_and_undo_unregisters() {
        let env_var = "AH_CRED_TEST_PLUGIN_KEY";
        with_env_vars(&[(env_var, "sk-plugin")], || {
            let ctx = Context::new();
            let plugin: DynPlugin = Arc::new(CredentialsPlugin::new(single_mapping(env_var)));
            let effects = ctx.mount(&plugin).expect("mount credentials plugin");

            let provider = ctx
                .service::<dyn CredentialProvider>(&CREDENTIALS)
                .expect("credentials seam registered");
            assert!(provider.get("test.api_key").is_some());

            // 卸载后 seam 回滚。
            drop(effects);
            assert!(!ctx.has_service(&CREDENTIALS));
        });
    }

    #[test]
    fn plugin_applies_with_custom_mapping_and_reads_env() {
        let env_var = "AH_CRED_TEST_CUSTOM_KEY";
        with_env_vars(&[(env_var, "sk-custom")], || {
            let ctx = Context::new();
            let mut mapping = HashMap::new();
            mapping.insert("my.key".to_string(), env_var.to_string());
            let plugin: DynPlugin = Arc::new(CredentialsPlugin::new(mapping));
            let _effects = ctx.mount(&plugin).expect("mount");

            let provider = ctx
                .service::<dyn CredentialProvider>(&CREDENTIALS)
                .expect("credentials seam registered");
            let credential = provider.get("my.key").expect("resolved via custom mapping");
            assert_eq!(credential.value, "sk-custom");
            assert_eq!(credential.source, format!("env:{env_var}"));
        });
    }
}
