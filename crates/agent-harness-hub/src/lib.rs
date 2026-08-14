//! # agent-harness-hub
//!
//! 插件内核(等价 Cordis 的最小面):
//! - ServiceRegistry:按类型化键注册/查找服务,支持依赖注入与循环依赖检测;
//! - EventBus:emit / waterfall / parallel / serial 四种分发;
//! - Effect:可逆注册的 RAII guard,drop 时自动回滚;
//! - Profile:组合配置层(bundle 顺序 + patch 覆盖)。
//!
//! 当前为脚手架骨架,接口与语义将在后续迭代中逐步落地。

/// 服务注册表:键 → 服务实例。
///
/// 骨架占位。后续将支持:
/// - 按 ServiceKey 注册/查找 Arc<dyn Any> 服务;
/// - 插件声明依赖键,注册时拓扑排序,循环依赖报错;
/// - 注册返回 Effect,卸载时自动反注册。
pub struct ServiceRegistry;

/// 事件总线:四种分发模式的骨架占位。
pub struct EventBus;

/// 可逆注册的 RAII guard(骨架占位)。
pub struct Effect;

/// Profile 组合配置(骨架占位)。
pub struct Profile;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_compiles() {
        // 脚手架冒烟测试:确保骨架可编译、可实例化。
        let _registry = ServiceRegistry;
        let _bus = EventBus;
        let _effect = Effect;
        let _profile = Profile;
    }
}
