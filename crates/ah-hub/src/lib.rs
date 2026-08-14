//! # ah-hub
//!
//! 插件内核(等价 Cordis 的最小面):
//! - [`ServiceRegistry`]:按类型化键注册/查找服务(Seam trait 对象),注册可逆;
//! - [`EventBus`]:emit / serial / parallel / waterfall 四种分发;
//! - [`Plugin`]:插件 trait + 依赖注入 + 拓扑排序挂载 + 循环依赖检测;
//! - [`Profile`]:组合配置层(bundle 顺序 + 插件清单),TOML 解析。
//!
//! [`Effect`](ah_contracts::Effect) 与 [`ServiceKey`](ah_contracts::service::ServiceKey)
//! 定义在契约层,此处再导出。

pub mod context;
pub mod event_bus;
pub mod plugin;
pub mod profile;
pub mod registry;

pub use ah_contracts::Effect;
pub use context::Context;
pub use event_bus::{EventBus, Next};
pub use plugin::{Plugin, PluginError};
pub use profile::{Bundle, Profile, ProfileError};
pub use registry::ServiceRegistry;
