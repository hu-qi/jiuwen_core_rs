# Service 与 Seam

一个完整的服务 Seam 由三部分组成：

1. `ah-contracts` 中的 Trait、事件和纯数据类型。
2. Provider 插件中的具体实现。
3. Consumer 插件中的契约调用。

Consumer 只通过 Context 解析 Trait Object，不导入 Provider 的具体类型。ServiceKey 是跨插件边界的稳定标识，应视为公开契约。

```rust
let provider = ctx
    .service::<dyn ModelProvider>(&ah_contracts::keys::LLM)
    .ok_or(PluginError::MissingDependency)?;
```

契约 crate 不放置 Mock、Unsupported、fallback 或业务逻辑。真实路径不支持时必须返回显式错误。
