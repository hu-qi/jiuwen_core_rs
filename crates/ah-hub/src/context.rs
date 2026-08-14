//! 插件上下文:服务注册表 + 事件总线。

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use ah_contracts::event::Event;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;

use crate::effect::Effect;
use crate::event_bus::{EventBus, Next};
use crate::plugin::{DynPlugin, PluginError};
use crate::registry::ServiceRegistry;

/// 插件上下文:服务注册表 + 事件总线。
///
/// 等价于 Cordis 的 Context。插件通过它注册服务、监听/发布事件;
/// 宿主通过 [`Context::mount_all`] 按 profile 组装插件树。
#[derive(Clone)]
pub struct Context {
    services: Arc<ServiceRegistry>,
    events: Arc<EventBus>,
}

impl Context {
    /// 创建空上下文。
    pub fn new() -> Self {
        Self {
            services: Arc::new(ServiceRegistry::new()),
            events: Arc::new(EventBus::new()),
        }
    }

    /// 注册一个服务并返回可逆注册 guard。
    pub fn register<S: Seam + ?Sized>(&self, key: ServiceKey, service: Arc<S>) -> Effect {
        self.services.register(key, service)
    }

    /// 按 Seam trait 对象取回服务。
    pub fn service<S: Seam + ?Sized>(&self, key: &ServiceKey) -> Option<Arc<S>> {
        self.services.get(key)
    }

    /// 服务键是否存在。
    pub fn has_service(&self, key: &ServiceKey) -> bool {
        self.services.has(key)
    }

    /// 当前注册的所有服务键。
    pub fn service_keys(&self) -> Vec<ServiceKey> {
        self.services.keys()
    }

    /// 注册 emit 监听器。
    pub fn on<E: Event>(&self, listener: impl Fn(E) + Send + Sync + 'static) -> Effect {
        self.events.on(listener)
    }

    /// 注册 serial 监听器。
    pub fn on_serial<E: Event, F, Fut>(&self, listener: F) -> Effect
    where
        F: Fn(E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.events.on_serial(listener)
    }

    /// 注册 parallel 监听器。
    pub fn on_parallel<E: Event, F, Fut>(&self, listener: F) -> Effect
    where
        F: Fn(E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.events.on_parallel(listener)
    }

    /// 注册 waterfall 监听器(收到 `(event, value, next)`)。
    pub fn on_waterfall<E: Event, R: Send + 'static, F, Fut>(&self, handler: F) -> Effect
    where
        F: Fn(E, R, Next<R>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        self.events.on_waterfall(handler)
    }

    /// emit:同步通知。
    pub fn emit<E: Event>(&self, event: E) {
        self.events.emit(event)
    }

    /// serial:按注册顺序 await。
    pub async fn serial<E: Event>(&self, event: E) {
        self.events.serial(event).await
    }

    /// parallel:并发执行并等待。
    pub async fn parallel<E: Event>(&self, event: E) {
        self.events.parallel(event).await
    }

    /// waterfall:委托链 / 短路。
    pub async fn waterfall<E: Event, R: Send + 'static>(&self, event: E, initial: R) -> R {
        self.events.waterfall(event, initial).await
    }

    /// 挂载单个插件:校验依赖与重复 provider,然后 apply。
    pub fn mount(&self, plugin: &DynPlugin) -> Result<Vec<Effect>, PluginError> {
        for key in plugin.inject() {
            if !self.has_service(&key) {
                return Err(PluginError::MissingDependency {
                    plugin: plugin.name(),
                    key,
                });
            }
        }
        for key in plugin.provides() {
            if self.has_service(&key) {
                return Err(PluginError::DuplicateProvider {
                    key,
                    provider: plugin.name(),
                });
            }
        }
        plugin.apply(self)
    }

    /// 批量挂载:按依赖拓扑排序,检测环与重复 provider。
    pub fn mount_all(&self, plugins: Vec<DynPlugin>) -> Result<Vec<Effect>, PluginError> {
        // 1) 收集组内 provider 索引,检测重复。
        let mut providers: HashMap<ServiceKey, usize> = HashMap::new();
        for (index, plugin) in plugins.iter().enumerate() {
            for key in plugin.provides() {
                if let Some(&other) = providers.get(&key) {
                    return Err(PluginError::DuplicateProvider {
                        key,
                        provider: plugins[other].name(),
                    });
                }
                providers.insert(key, index);
            }
        }

        // 2) DFS 拓扑排序(依赖在前),检测环。
        const WHITE: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;
        let mut state = vec![WHITE; plugins.len()];
        let mut order: Vec<usize> = Vec::new();
        let mut path: Vec<&'static str> = Vec::new();

        fn visit(
            index: usize,
            plugins: &[DynPlugin],
            providers: &HashMap<ServiceKey, usize>,
            state: &mut [u8],
            order: &mut Vec<usize>,
            path: &mut Vec<&'static str>,
        ) -> Result<(), PluginError> {
            match state[index] {
                GRAY => {
                    path.push(plugins[index].name());
                    return Err(PluginError::CycleDetected {
                        chain: path.clone(),
                    });
                }
                BLACK => return Ok(()),
                _ => {}
            }
            state[index] = GRAY;
            path.push(plugins[index].name());
            for key in plugins[index].inject() {
                if let Some(&dep) = providers.get(&key) {
                    visit(dep, plugins, providers, state, order, path)?;
                }
            }
            path.pop();
            state[index] = BLACK;
            order.push(index);
            Ok(())
        }

        for index in 0..plugins.len() {
            visit(
                index, &plugins, &providers, &mut state, &mut order, &mut path,
            )?;
        }

        // 3) 按拓扑序挂载(依赖先于消费者)。
        let mut effects = Vec::new();
        for index in order {
            effects.extend(self.mount(&plugins[index])?);
        }
        Ok(effects)
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Plugin;
    use std::sync::Mutex;

    /// 一个可记录 apply 顺序的桩服务。
    struct Marker;
    impl Seam for Marker {}

    struct RecordingPlugin {
        name: &'static str,
        provides_key: &'static str,
        inject_keys: Vec<&'static str>,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Plugin for RecordingPlugin {
        fn name(&self) -> &'static str {
            self.name
        }

        fn provides(&self) -> Vec<ServiceKey> {
            vec![ServiceKey::new(self.provides_key)]
        }

        fn inject(&self) -> Vec<ServiceKey> {
            self.inject_keys
                .iter()
                .map(|k| ServiceKey::new(k))
                .collect()
        }

        fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
            self.log.lock().unwrap().push(self.name);
            let effect = ctx.register(ServiceKey::new(self.provides_key), Arc::new(Marker));
            Ok(vec![effect])
        }
    }

    fn plugin(
        name: &'static str,
        provides_key: &'static str,
        inject_keys: Vec<&'static str>,
        log: &Arc<Mutex<Vec<&'static str>>>,
    ) -> DynPlugin {
        Arc::new(RecordingPlugin {
            name,
            provides_key,
            inject_keys,
            log: log.clone(),
        })
    }

    #[test]
    fn mount_all_respects_dependency_order() {
        let ctx = Context::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        // 故意乱序传入:a 依赖 b,b 依赖 c。
        let a = plugin("a", "a", vec!["b"], &log);
        let b = plugin("b", "b", vec!["c"], &log);
        let c = plugin("c", "c", vec![], &log);

        ctx.mount_all(vec![a, b, c]).expect("mount should succeed");
        assert_eq!(*log.lock().unwrap(), vec!["c", "b", "a"]);
    }

    #[test]
    fn mount_all_detects_duplicate_provider() {
        let ctx = Context::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = plugin("a", "x", vec![], &log);
        let b = plugin("b", "x", vec![], &log);

        let error = ctx.mount_all(vec![a, b]).expect_err("duplicate provider");
        assert!(
            matches!(&error, PluginError::DuplicateProvider { key, provider } if key.name() == "x" && *provider == "a")
        );
    }

    #[test]
    fn mount_all_detects_cycle() {
        let ctx = Context::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = plugin("a", "a", vec!["b"], &log);
        let b = plugin("b", "b", vec!["a"], &log);

        let error = ctx.mount_all(vec![a, b]).expect_err("cycle");
        assert!(matches!(&error, PluginError::CycleDetected { .. }));
    }

    #[test]
    fn mount_missing_dependency_fails() {
        let ctx = Context::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = plugin("a", "a", vec!["nonexistent"], &log);

        let error = ctx.mount_all(vec![a]).expect_err("missing dep");
        assert!(matches!(
            &error,
            PluginError::MissingDependency { plugin, key } if *plugin == "a" && key.name() == "nonexistent"
        ));
    }

    #[test]
    fn mount_effect_undo_unregisters_service() {
        let ctx = Context::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let a = plugin("a", "a", vec![], &log);
        let effects = ctx.mount_all(vec![a]).expect("mount");
        assert!(ctx.has_service(&ServiceKey::new("a")));

        drop(effects);
        assert!(!ctx.has_service(&ServiceKey::new("a")));
    }
}
