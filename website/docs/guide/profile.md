# Profile 组合

Profile 是 Bundle 和插件名清单。它把环境选择放在配置中，不要求修改插件代码。

```toml
name = "dev"

[[bundles]]
id = "mock"
plugins = ["ah-plugins-mock"]
```

## 挂载规则

- `provides` 声明服务输出。
- `inject` 声明服务依赖。
- 挂载顺序由依赖拓扑决定，不由 TOML 顺序决定。
- 缺失依赖、依赖环和重复 Provider 必须失败。
- 生产 Profile 不得包含 Mock 插件。

## 组合验证

对每个新 Profile 验证：

1. 所有插件名称都能被 Catalog 解析。
2. 依赖图可排序且没有环。
3. 所有服务都能通过 Context 解析。
4. 任一插件挂载失败时，之前的注册会回滚。
5. 释放全部 Effect 后，服务和监听器不再可见。
