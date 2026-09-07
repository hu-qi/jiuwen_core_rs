# 事件监听

EventBus 支持四种分发模式：

| 模式 | 语义 | 适用场景 |
| --- | --- | --- |
| `emit` | 同步、按注册顺序 | 日志和轻量观察 |
| `serial` | 异步、顺序等待 | 有序副作用 |
| `parallel` | 异步、并发执行 | 独立观察者 |
| `waterfall` | 异步、`next()` 委托链 | rails 和决策链 |

## 注册监听器

```rust
let effect = ctx.on::<MyEvent>(|event| {
    println!("observed: {:?}", event);
});
ctx.emit(MyEvent::new());
drop(effect);
```

## Waterfall

监听器只有调用 `next()` 才会委托给下游；不调用 `next()` 即短路。Effect 释放后，监听器立即移除。

事件类型应实现 `Event` 契约，并提供稳定 ID。事件生产者、消费者和持久化关系见 `docs/event-catalog.md`。
