//! # agent-harness-contracts
//!
//! 契约层:只声明接口(Seam trait)与纯类型,零实现。
//!
//! 设计规则(对齐 DSH/Cordis):
//! - 本 crate 不允许出现 Mock / Unsupported / fallback 实现;
//!   生产路径的实现必须来自插件 crate。
//! - 每个 Seam 由三角构成:Service Definition(接口)、Service Provider(实现)、
//!   Consumer(消费方,通常是模型可见工具)。
//! - 插件之间不允许直接依赖彼此的具体类型,只允许依赖本 crate 的契约。
//!
//! 契约版本占位:后续按 seam 逐域展开
//! (llm / tools / session / workflow / context / memory / retrieval /
//!  fs / shell / sandbox / security / teams / evolving / rsi / telemetry / transport)。

pub mod prelude {
    pub use crate::service::ServiceKey;
}

/// 服务注册表使用的类型化键。
///
/// 等价于 Cordis 的 ctx.<key>:插件按键查找服务,而不是 import 具体实现。
pub mod service {
    /// 类型化服务键:同一个键在注册表中唯一对应一个服务实例。
    pub struct ServiceKey {
        /// 稳定的服务名,如 llm、tools、sessions。
        name: &'static str,
    }

    impl ServiceKey {
        /// 构造一个新的服务键。
        pub const fn new(name: &'static str) -> Self {
            Self { name }
        }

        /// 返回服务键名。
        pub const fn name(&self) -> &'static str {
            self.name
        }
    }

    impl core::fmt::Display for ServiceKey {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(self.name)
        }
    }

    impl core::fmt::Debug for ServiceKey {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_tuple("ServiceKey").field(&self.name).finish()
        }
    }

    impl PartialEq for ServiceKey {
        fn eq(&self, other: &Self) -> bool {
            self.name == other.name
        }
    }

    impl Eq for ServiceKey {}

    impl core::hash::Hash for ServiceKey {
        fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
            self.name.hash(state);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn service_key_identity() {
            let llm = ServiceKey::new("llm");
            assert_eq!(llm, ServiceKey::new("llm"));
            assert_ne!(llm, ServiceKey::new("tools"));
            assert_eq!(llm.to_string(), "llm");
        }
    }
}
