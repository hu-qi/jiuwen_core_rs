# 插件与运行时组合

`agent-harness` 使用 Plugin、Seam、EventBus、Effect 和 Profile 组合运行时。

## 组合关系

```text
Profile → Plugin Catalog → Dependency Graph → Mount → Service Resolve → Invoke
```

Provider 只实现契约；Consumer 只通过 `Context` 解析契约。插件之间禁止直接依赖其他插件的具体类型。

## 生命周期

插件的注册、事件监听器和后台资源必须绑定 `Effect`。卸载或关闭时释放 Effect，注册即可回滚；插件创建的后台任务、子进程和 Transport 也必须显式终止。

## Profile 规则

- 挂载顺序由 `provides/inject` 依赖拓扑决定。
- 缺失依赖、依赖环和重复 Provider 必须失败。
- 生产 Profile 不得包含 Mock 插件。
- 静态编译、启动时组合和关闭时释放是当前插件模型。

完整语义见 `agent-harness/docs/architecture.md` 和 `agent-harness/docs/hub-primer.md`。
