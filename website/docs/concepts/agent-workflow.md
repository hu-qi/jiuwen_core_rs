# Agent Loop 与 Workflow 插件

Harness 将 Agent Loop 和 Workflow 作为可组合插件能力。

## Agent Loop

Agent Loop 负责协调模型请求、工具调用、事件和终止条件。Consumer 通过服务契约取得模型与工具，不直接依赖 Provider 实现。

## Workflow

Workflow 插件负责图结构、组件执行、分支、循环和状态恢复。应用层应通过 Application 或 Controller Seam 调用，不在插件之间传递具体内部类型。

## 组合原则

- 动态决策放入 Agent Loop。
- 固定步骤和可审计流程放入 Workflow。
- 长任务必须定义 Checkpoint、取消和终止状态。
- 生产行为要有真实 Provider 或后端验证。
