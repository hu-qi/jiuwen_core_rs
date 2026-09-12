# 事件目录(event-catalog.md)

> 等价 DSH 的 event-producer-consumer:登记每个事件的类型、分发模式、生产者、消费者。
> 事件一旦被插件使用,条目不得删除,只能标注 deprecated。
>
> 当前已实现 **6 个契约级事件**(全部定义于 ah-contracts/src,Event trait 实现于
> agent.rs / session.rs / tools.rs / swarm.rs / teams.rs)。分发模式与生产者/消费者
> 均以 crates/ 下代码为准(ah-hub EventBus:emit / serial / parallel / waterfall)。

## 已实现事件(6)

| 事件 | 分发模式 | 生产者(插件/调用点) | 消费者(监听器) | 状态 |
| --- | --- | --- | --- | --- |
| agent/step | emit | ah-plugins-agent-loop(AgentLoop 每轮结束 emit AgentStep) | ah-plugins-telemetry(span 记录)、ah-app(demo 打印) | **done(已实现)** |
| session/event | emit | ah-plugins-session-log(JsonlSessionLog::append 落盘后广播) | ah-app CLI(ah-cli 终端展示);投影/回放/持久化(session-log 内部) | **done(已实现)** |
| tools/pre-execute | waterfall | ah-plugins-tools(ToolRegistry::invoke 执行前,可拒绝/改写) | ah-plugins-rails(ShellGuard/PathGuard/ToolBudget/Approval)、ah-plugins-security、ah-plugins-sandbox | **done(已实现)** |
| tools/post-execute | serial | ah-plugins-tools(ToolRegistry::invoke 执行后) | ah-plugins-telemetry(工具 span + JSONL 导出) | **done(已实现)** |
| teams/swarm | emit | ah-plugins-teams(SwarmflowRunner,swarm 引擎进度) | 观察者折叠为 SwarmRun(swarm.rs 内部)、UI/干跑 | **done(已实现)** |
| teams/task | emit | ah-plugins-teams(InMemoryTeamRuntime / SqliteTeamRuntime,状态迁移时) | 任务板、审计(暂无外部监听器注册) | **done(已实现)** |

> 注:ah-plugins-workflow 内部另定义并 emit workflow/node(WorkflowNodeEvent,
> 插件本地事件,非 ah-contracts 契约级事件),不计入上表 6 个登记事件。

## 事件数据结构(字段,定义于 ah-contracts/src)

### agent/step — AgentStep(agent.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| iteration | usize | 第几轮(0 起) |
| tool_calls | usize | 本轮模型请求的工具调用数 |
| done | bool | 是否已得到最终回答(循环结束) |

### session/event — SessionEvent(session.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| seq | u64 | 单调递增序号(append-only) |
| timestamp_ms | u64 | 时间戳 |
| kind | SessionEventKind | User / Assistant / ToolResult / System / AgentStep |
| payload | Value | JSON 负载(投影 derive_messages 依赖,约定见下) |

payload 约定:
- User / Assistant(无 tool_calls):{"content": string}
- Assistant 工具调用:{"tool_calls": [{id, name, arguments}]}
- ToolResult:{"tool_call_id": string, "output": string}
- AgentStep:{"iteration": int, "tool_calls": int, "done": bool}

### tools/pre-execute — ToolInvocation(tools.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| name | String | 工具名 |
| arguments | Value | 参数(监听器可改写) |
| context | ToolInvocationContext | 请求级 session_id 与可选 caller identity；不进入全局工具 schema |

waterfall 决策值 ToolDecision { allow: bool, reason: Option<String>, arguments: Value }:
监听器调 Next::next 委托下游(可改写参数);直接返回决策即短路——allow=false 时
工具不执行,ToolRegistry::invoke 返回 "rejected by rail: {reason}"。

### tools/post-execute — ToolExecuted(tools.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| name | String | 工具名 |
| arguments | Value | 实际参数 |
| output | Value | 执行结果 |
| elapsed_ms | u64 | 耗时(毫秒) |
| context | ToolInvocationContext | 与 pre-execute 和实际工具调用相同的请求归属上下文 |

### teams/swarm — SwarmEvent(swarm.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| run_id | String | 运行 id(journal 续跑按同名短路) |
| kind | SwarmEventKind | WorkflowStarted / PhaseStarted(String) / AgentStarted(String) / AgentCompleted(String, bool) / WorkflowCompleted(bool) |

### teams/task — TeamTaskEvent(teams.rs)

| 字段 | 类型 | 语义 |
| --- | --- | --- |
| team | String | 团队 id |
| task_id | String | 任务 id |
| status | TeamTaskStatus | Pending / InProgress / InReview / Done / Failed |

## 规划事件(未实现,按 seam 落地顺序登记)

| 事件 | 分发模式 | 生产者 | 消费者 | 状态 |
| --- | --- | --- | --- | --- |
| session/step | emit | agent 循环 | 日志、遥测、UI | 规划(由 agent/step + session/event 组合覆盖,不再单独规划) |
| agent/pre-step | waterfall | agent 循环 | rails、上下文注入 | 规划 |
| agent/request | waterfall | agent 循环 | 模型适配、拦截 | 规划 |
| fs/* | waterfall | fs seam | 沙箱、策略 | 规划(路径策略经 tools/pre-execute:PathGuardRail/SandboxRail 已落地) |
| telemetry/* | emit | 各插件 | 导出器 | span 记录与 JSONL、OTLP/JSON HTTP 导出已实现;完整 OTel SDK、重试/批处理和 collector production E2E 仍为 partial |

## 事件登记规则

1. 事件类型实现 Event trait(Clone,稳定 ID);
2. 按语义选模式:观察 emit / 顺序副作用 serial / 扇出 parallel / 决策链 waterfall;
3. waterfall 事件必须文档标注语义(是否允许短路、短路含义);
4. 新增事件在本表登记后,才允许在生产代码中发布。
