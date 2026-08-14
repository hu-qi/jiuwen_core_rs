# 事件目录(event-catalog.md)

> 等价 DSH 的 event-producer-consumer:登记每个事件的类型、分发模式、生产者、消费者。
> 事件一旦被插件使用,条目不得删除,只能标注 deprecated。

## 已实现事件

当前内核仅测试事件(Ping),无生产事件。事件按能力落地逐步登记到下表。

## 规划事件(按 seam 落地顺序登记)

| 事件 | 分发模式 | 生产者 | 消费者 | 状态
| --- | --- | --- | --- | --- |
| session/step | emit | agent 循环 | 日志、遥测、UI | 规划(会话子系统)
| session/event | emit | 会话存储 | 投影、回放、持久化 | 规划(日志即真相)
| agent/pre-step | waterfall | agent 循环 | rails、上下文注入 | 规划(agent-loop)
| agent/request | waterfall | agent 循环 | 模型适配、拦截 | 规划
| tools/pre-execute | waterfall | 工具执行管线 | 策略、鉴权、超时 | 规划(tools seam)
| tools/post-execute | serial | 工具执行管线 | 遥测、审计 | 规划
| fs/* | waterfall | fs seam | 沙箱、策略 | 规划
| telemetry/* | emit | 各插件 | 导出器 | 规划(telemetry seam)
| teams/task | serial | teams seam | 任务板、审计 | 规划(teams)

## 事件登记规则

1. 事件类型实现 Event trait(Clone,稳定 ID);
2. 按语义选模式:观察 emit / 顺序副作用 serial / 扇出 parallel / 决策链 waterfall;
3. waterfall 事件必须文档标注语义(是否允许短路、短路含义);
4. 新增事件在本表登记后,才允许在生产代码中发布。
