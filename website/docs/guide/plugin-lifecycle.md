# 插件生命周期与测试

插件生命周期由宿主控制，插件实现只负责声明依赖并返回它创建的资源 Effect。

## 生命周期

```text
创建插件对象
  ↓
Catalog 解析名称
  ↓
Profile 展开插件列表
  ↓
mount_all 校验并排序
  ↓
Plugin::apply 注册服务/监听器
  ↓
Context 解析并调用服务
  ↓
释放 Effect，撤销注册
```

## `mount_all` 的执行规则

1. 收集所有 `provides()`，同一 ServiceKey 重复时返回 `DuplicateProvider`。
2. 根据 `inject()` 构建依赖关系并进行拓扑排序。
3. 发现依赖环时返回 `CycleDetected`。
4. 发现组内和已注册 Context 中都不存在的依赖时返回 `MissingDependency`。
5. 依赖满足后按拓扑顺序调用每个插件的 `apply()`。
6. 任一 `apply()` 失败时释放已经成功注册的 Effect，恢复挂载前状态。

依赖顺序由插件声明决定，不由 Profile 或调用方传入的数组顺序决定。

## Effect 管理

```rust
fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
    let service_effect = ctx.register(KEY, Arc::new(MyProvider::new()));
    let listener_effect = ctx.on::<MyEvent>(handle_event);
    Ok(vec![service_effect, listener_effect])
}
```

必须返回全部 Effect。`let _ = ctx.register(...)` 会立即释放临时 guard，导致服务马上被撤销。

## 测试矩阵

| 场景 | 应验证 |
| --- | --- |
| 正常挂载 | `apply` 被调用，服务可解析 |
| 依赖排序 | 消费者在 Provider 之后挂载 |
| 重复 Provider | 返回 `PluginError::DuplicateProvider` |
| 缺失依赖 | 返回 `PluginError::MissingDependency` |
| 依赖环 | 返回 `PluginError::CycleDetected` |
| apply 失败 | 之前的服务和监听器全部回滚 |
| 卸载 | drop 或 `dispose()` 后注册消失 |
| 资源清理 | 后台任务、进程、文件和连接被终止 |

当前实现已有对应测试：`ah-hub/src/context.rs` 中的 `mount_all_*` 测试，以及 `ah-app/tests/stage2_plugins.rs` 的 `catalog_plugins_mount_resolve_invoke_and_unmount`。
