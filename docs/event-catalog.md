# 事件目录(event-catalog.md)

> 等价 DSH 的 event-producer-consumer:登记每个事件的类型、分发模式、生产者、消费者。
> 事件一旦被插件使用,条目不得删除,只能标注 deprecated。

## 已实现事件

当前内核仅测试事件(Ping),无生产事件。事件按能力落地逐步登记到下表。

## 规划事件(按 seam 落地顺序登记)

| 事件 | 分发模式 | 生产者 | 消费者 | 状态
| --- | --- | --- | --- | --- |
| session/event | emit | SessionLog.append | 投影、回放、持久化、UI | **done(已实现)**
| session/step | emit | agent 循环 | 日志、遥测、UI | 规划(由 agent/step + session/event 组合覆盖)
| agent/step | emit | AgentLoop | 遥测、日志、UI | **done(已实现)**
| agent/pre-step | waterfall | agent 循环 | rails、上下文注入 | 规划
| agent/request | waterfall | agent 循环 | 模型适配、拦截 | 规划
| tools/pre-execute | waterfall | LocalToolRegistry.invoke | rails、鉴权、参数改写 | **done(已实现)**
| tools/post-execute | serial | LocalToolRegistry.invoke | 遥测、审计 | **done(已实现)**
| fs/* | waterfall | fs seam | 沙箱、策略 | 规划(路径策略经 tools/pre-execute:PathGuardRail/SandboxRail 已落地) |
| telemetry/* | emit | 各插件 | 导出器 | 规划(telemetry seam 已落地:span 记录 + JSONL 导出,由 agent/step 与 tools/post-execute 驱动;OTLP 留待后续)
| teams/task | serial | InMemoryTeamRuntime / SqliteTeamRuntime(状态迁移时 emit) | 任务板、审计 | **done(已实现)**

## 事件登记规则

1. 事件类型实现 Event trait(Clone,稳定 ID);
2. 按语义选模式:观察 emit / 顺序副作用 serial / 扇出 parallel / 决策链 waterfall;
3. waterfall 事件必须文档标注语义(是否允许短路、短路含义);
4. 新增事件在本表登记后,才允许在生产代码中发布。
| workflow/node | emit | WorkflowEngineImpl.execute_node | 遥测、审计 | **done(已实现)** |
