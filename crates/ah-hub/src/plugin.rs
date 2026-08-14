//! 插件 trait 与挂载错误。

use std::sync::Arc;

use ah_contracts::service::ServiceKey;

use crate::context::Context;
use ah_contracts::Effect;

/// 插件:挂载到 Context 上,注册服务与事件。
///
/// 对齐 DSH/Cordis:插件声明 `provides`(提供的服务键)与 `inject`(依赖的服务键),
/// 加载顺序由服务依赖表达,而非手工 boot 排序。
pub trait Plugin: Send + Sync {
    /// 插件稳定名称(profile 中引用)。
    fn name(&self) -> &'static str;

    /// 本插件提供的服务键。
    fn provides(&self) -> Vec<ServiceKey>;

    /// 本插件依赖的服务键(在 apply 前必须已存在)。
    fn inject(&self) -> Vec<ServiceKey> {
        Vec::new()
    }

    /// 挂载动作:注册服务/事件,返回可逆注册 guard 列表。
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError>;
}

/// 插件挂载错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// 依赖的服务键在挂载时不存在。
    MissingDependency {
        plugin: &'static str,
        key: ServiceKey,
    },
    /// 同一服务键被多个 provider 声明。
    DuplicateProvider {
        key: ServiceKey,
        provider: &'static str,
    },
    /// 依赖图中检测到环。
    CycleDetected { chain: Vec<&'static str> },
    /// 插件 apply 内部错误。
    Apply {
        plugin: &'static str,
        message: String,
    },
}

impl core::fmt::Display for PluginError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingDependency { plugin, key } => {
                write!(f, "plugin `{plugin}` requires missing service `{key}`")
            }
            Self::DuplicateProvider { key, provider } => {
                write!(
                    f,
                    "service `{key}` provided twice (duplicate from `{provider}`)"
                )
            }
            Self::CycleDetected { chain } => {
                write!(f, "plugin dependency cycle: {:?}", chain)
            }
            Self::Apply { plugin, message } => {
                write!(f, "plugin `{plugin}` apply failed: {message}")
            }
        }
    }
}

impl std::error::Error for PluginError {}

/// 便捷:trait 对象别名。
pub type DynPlugin = Arc<dyn Plugin>;
