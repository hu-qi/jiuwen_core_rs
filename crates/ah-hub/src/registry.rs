//! 服务注册表。

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;

use crate::effect::Effect;

/// 服务注册表:按类型化键存储服务。
///
/// 等价于 Cordis 的 `ctx`:插件按键注册/查找服务,而不是 import 具体实现。
/// 存储的载荷是 `Arc<dyn Any>`(内部为 `Arc<S>`),查找时按 Seam trait 对象下转。
#[derive(Default)]
pub struct ServiceRegistry {
    services: RwLock<HashMap<ServiceKey, Arc<dyn Any + Send + Sync>>>,
}

impl ServiceRegistry {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个服务并返回可逆注册 guard。
    ///
    /// `S` 可以是具体类型或 trait 对象(如 `dyn ModelProvider`)。
    /// 消费方用同样的 `S` 调用 [`ServiceRegistry::get`] 取回。
    ///
    /// 注意:同一键重复注册会覆盖旧值;插件挂载层负责拒绝重复 provider。
    pub fn register<S: Seam + ?Sized>(
        self: &Arc<Self>,
        key: ServiceKey,
        service: Arc<S>,
    ) -> Effect {
        let payload: Arc<dyn Any + Send + Sync> = Arc::new(service);
        {
            let mut guard = self.services.write().expect("registry poisoned");
            guard.insert(key, payload);
        }
        let registry = Arc::clone(self);
        Effect::new(move || {
            registry
                .services
                .write()
                .expect("registry poisoned")
                .remove(&key);
        })
    }

    /// 按 Seam trait 对象取回服务。
    pub fn get<S: Seam + ?Sized>(&self, key: &ServiceKey) -> Option<Arc<S>> {
        let payload = self.services.read().ok()?.get(key)?.clone();
        let inner = payload.downcast::<Arc<S>>().ok()?;
        Some(inner.as_ref().clone())
    }

    /// 键是否存在(无论类型)。
    pub fn has(&self, key: &ServiceKey) -> bool {
        self.services
            .read()
            .ok()
            .map(|guard| guard.contains_key(key))
            .unwrap_or(false)
    }

    /// 当前注册的所有键。
    pub fn keys(&self) -> Vec<ServiceKey> {
        self.services
            .read()
            .ok()
            .map(|guard| guard.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::{ModelError, ModelProvider, ModelRequest, ModelResponse};

    struct FakeProvider;

    impl Seam for FakeProvider {}

    #[async_trait::async_trait]
    impl ModelProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn chat(&self, _request: ModelRequest) -> Result<ModelResponse, ModelError> {
            Ok(ModelResponse {
                content: "fake reply".to_string(),
            })
        }
    }

    #[test]
    fn register_and_get_trait_object() {
        let registry = Arc::new(ServiceRegistry::new());
        let key = ServiceKey::new("llm");
        let provider: Arc<dyn ModelProvider> = Arc::new(FakeProvider);
        let effect = registry.register(key, provider);

        let resolved: Arc<dyn ModelProvider> = registry.get(&key).expect("service missing");
        assert_eq!(resolved.name(), "fake");
        assert!(registry.has(&key));

        drop(effect);
        assert!(!registry.has(&key));
        assert!(registry.get::<dyn ModelProvider>(&key).is_none());
    }

    #[test]
    fn get_with_wrong_trait_returns_none() {
        let registry = Arc::new(ServiceRegistry::new());
        let key = ServiceKey::new("llm");
        let provider: Arc<dyn ModelProvider> = Arc::new(FakeProvider);
        let _effect = registry.register(key, provider);

        // 用一个不匹配的 Seam trait 查询应得到 None,而不是 panic。
        struct WrongSeam;
        impl Seam for WrongSeam {}
        assert!(registry.get::<WrongSeam>(&key).is_none());
    }

    #[test]
    fn register_overwrites_and_undo_removes() {
        let registry = Arc::new(ServiceRegistry::new());
        let key = ServiceKey::new("kv");
        let first: Arc<dyn ModelProvider> = Arc::new(FakeProvider);
        let e1 = registry.register(key, first);

        // 同键覆盖
        let second: Arc<dyn ModelProvider> = Arc::new(FakeProvider);
        let e2 = registry.register(key, second);
        assert!(registry.has(&key));

        // 撤销第二个注册后,键应被移除(注册表语义:覆盖后旧值丢失)。
        drop(e2);
        assert!(!registry.has(&key));
        drop(e1);
        assert!(!registry.has(&key));
    }
}
