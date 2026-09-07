# Plugin API

本页是 `agent-harness` 插件开发所需的核心 Rust API 参考。代码事实以 `ah-hub`、`ah-contracts` 当前源码为准。

## `Plugin`

定义位置：`crates/ah-hub/src/plugin.rs`。

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn provides(&self) -> Vec<ServiceKey>;
    fn inject(&self) -> Vec<ServiceKey> { Vec::new() }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError>;
}

pub type DynPlugin = Arc<dyn Plugin>;
```

| 方法 | 要求 |
| --- | --- |
| `name()` | 返回 Profile 和 Catalog 使用的稳定名称；同一插件实例不应根据运行时状态改变名称 |
| `provides()` | 返回本插件注册的所有 ServiceKey；用于重复 Provider 检查和依赖排序 |
| `inject()` | 返回 `apply()` 前必须可解析的 ServiceKey；默认返回空列表 |
| `apply()` | 执行注册并返回全部 Effect；失败必须返回 `PluginError::Apply` 或其他明确错误 |

插件必须实现 `Send + Sync`。插件之间不应通过具体类型互相引用。

## `Context`

定义位置：`crates/ah-hub/src/context.rs`。

| API | 语义 |
| --- | --- |
| `Context::new()` | 创建空 Context |
| `register(key, Arc<S>) -> Effect` | 注册一个 `Seam` 服务；Effect 释放时撤销 |
| `service::<S>(&key) -> Option<Arc<S>>` | 按 ServiceKey 解析 Trait Object |
| `has_service(&key) -> bool` | 检查服务是否存在 |
| `service_keys() -> Vec<ServiceKey>` | 获取当前所有服务键，用于诊断 |
| `mount(plugin) -> Result<Vec<Effect>, PluginError>` | 校验并挂载单个插件 |
| `mount_all(plugins) -> Result<Vec<Effect>, PluginError>` | 拓扑排序后挂载插件组，失败时回滚 |
| `on::<E>(handler) -> Effect` | 注册同步事件监听器 |
| `on_serial::<E, F, Fut>(handler) -> Effect` | 按注册顺序异步通知 |
| `on_parallel::<E, F, Fut>(handler) -> Effect` | 并发通知全部监听器 |
| `on_waterfall::<E, R, F, Fut>(handler) -> Effect` | 注册可通过 `next()` 委托或短路的监听器 |
| `emit(event)` | 同步发布事件 |
| `serial(event).await` | 顺序发布事件 |
| `parallel(event).await` | 并发发布事件 |
| `waterfall(event, initial).await` | 执行 waterfall 链并返回最终值 |

## `ServiceKey`

定义位置：`crates/ah-contracts/src/service.rs`。

```rust
pub const LLM: ServiceKey = ServiceKey::new("llm");
let custom = ServiceKey::new("my-service");
assert_eq!(custom.name(), "my-service");
```

ServiceKey 以字符串作为身份。同一 Context 中同名键只能对应一个 Provider；名称一旦被多个插件共享，就属于跨插件契约的一部分。

## `Effect`

定义位置：`crates/ah-contracts/src/effect.rs`。

```rust
let effect = Effect::new(|| println!("undo"));
effect.dispose();
let _inert = Effect::noop();
```

- `drop(effect)` 自动执行一次回滚。
- `effect.dispose()` 消费 guard 并执行一次回滚。
- 回滚闭包只能执行一次。
- 插件返回的 Effect 必须由宿主或插件生命周期持有到卸载。

## `PluginError`

| 变体 | 触发条件 |
| --- | --- |
| `MissingDependency { plugin, key }` | `inject()` 声明的服务不存在 |
| `DuplicateProvider { key, provider }` | ServiceKey 被多个 Provider 声明或注册 |
| `CycleDetected { chain }` | 插件依赖图存在环 |
| `Apply { plugin, message }` | `apply()` 内部失败 |

错误应直接暴露给启动器或调用方，不得用另一个 Provider 静默掩盖。
