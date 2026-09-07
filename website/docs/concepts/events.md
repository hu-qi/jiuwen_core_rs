# 事件与 Effect

## EventBus

事件是插件扩展点。`emit` 用于同步通知，`serial` 保证异步监听器顺序，`parallel` 并发执行监听器，`waterfall` 通过 `next()` 组成可短路的决策链。

## Effect

服务注册、事件监听和后台资源都必须绑定 Effect：

```rust
let registration = ctx.register(key, service);
let listener = ctx.on::<MyEvent>(handler);
// 保留 registration 和 listener，直到插件关闭
```

Effect 被释放时撤销对应注册。插件 `apply` 返回的 Effect 集合由 Harness 持有；挂载失败或关闭时释放集合即可回滚。

`let _ = ...` 会立即释放临时 Effect，是常见错误。事件模式和监听器关系见 `docs/event-catalog.md`。
