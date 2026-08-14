//! 类型化服务键,等价于 Cordis 的 `ctx.<key>`。

/// 类型化服务键:同一个键在注册表中唯一对应一个服务实例。
///
/// 插件按键查找服务,而不是 import 具体实现。
#[derive(Clone, Copy)]
pub struct ServiceKey {
    /// 稳定的服务名,如 `llm`、`tools`、`sessions`。
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
        assert_eq!(llm.name(), "llm");
    }
}
