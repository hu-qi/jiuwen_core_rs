# Harness 核心概念

`agent-harness` 的内核只负责组合机制，不内置业务 Provider。运行时由以下部件组成：

```text
Contracts → Hub → Plugins → Profile → Application
```

- **Contracts**：定义 Seam、事件和跨插件纯类型。
- **Hub**：提供 Registry、EventBus、Effect、Plugin 和 Profile 机制。
- **Plugins**：实现模型、工具、会话、工作流、团队、遥测等能力。
- **Profile**：声明插件清单和环境组合。
- **Application**：读取 Profile、解析 Catalog、挂载插件并启动服务。

开始阅读：[Service 与 Seam](/concepts/services)、[事件与 Effect](/concepts/events)、[Profile 组合](/guide/profile)。
