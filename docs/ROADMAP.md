# 当前路线图

> 基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`。
> 本文是当前执行顺序的唯一来源。能力明细见 `capability-map.md`,严格审计见
> `parity-audit.md`,历史回合记录见 `REMAINING_PLAN.md`。

## 完成口径

每个任务必须同时记录:

- 实现位置和公开行为面;
- 聚焦测试及命令;
- production profile/真实协议验证结果;
- Python/Rust differential 状态;
- 未验证项和外部依赖。

Golden fixture、mock 测试或 Rust 自生成 reference 只能证明 Rust 内部回归稳定,不能单独把
Python parity 标记为 verified。

## P0 可信验收与生产可运行性

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P0-01 | 建立真实 Python/Rust differential runner | missing | 同一语言中立 fixture 分别驱动 Python 与 Rust;CI 比较输出、错误、状态、日志、恢复、取消和超时 |
| P0-02 | production profile 静态组合验证 | missing | profile 每个插件可由 catalog 解析;provides/inject 无缺失、重复和环 |
| P0-03 | production boot smoke | missing | 无 mock,使用本地协议 fixture 和临时持久化目录完成 boot 与一次 `ApplicationRuntime::invoke` |
| P0-04 | `mount_all` 失败原子性验证 | partial | 后续插件 apply 失败后,此前服务和事件监听器全部回滚,Context 回到调用前状态 |
| P0-05 | 统一审计数据源 | partial | done/partial/missing、域汇总和百分比由结构化数据生成,不再手工累计 |
| P0-06 | 文档与 CI 事实对齐 | in progress | README、testing、development、审计和技术目录不再引用过期数字或不存在的门禁 |

首批 differential 范围:application、agent-loop、session、controller、workflow、tools。

## P1 核心 Agent 主链与插件生命周期

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P1-01 | 宿主只消费 `dyn AgentLoopRuntime` | partial | `ah-app`/CLI 不解析具体 `AgentLoop`;替换 provider 无需修改宿主 |
| P1-02 | 结构化运行错误和真实统计 | partial | 取消/中断/超时不依赖错误字符串;AgentResult 返回真实 iterations、tool calls 和终止原因 |
| P1-03 | timeout/cancel/interrupt 全链传播 | partial | 可中断阻塞中的模型、流式和工具调用;事件顺序与恢复行为通过 differential |
| P1-04 | session/checkpoint 完整恢复 | partial | 崩溃、部分 tool call、fork/restore、幂等、版本兼容和并发访问均有契约测试 |
| P1-05 | application 完整绑定 | partial | 结构化 command、LLM intent、memory/invoke rails 和请求级配置与 Python 公开行为对等 |
| P1-06 | controller 完整行为 | partial | LLM intent、状态机、父子任务、并发调度和 snapshot 迁移通过 differential |
| P1-07 | workflow 流式执行 | partial | STREAM/TRANSFORM/COLLECT、增量工具结果、中断和 checkpoint 续跑完整接线 |
| P1-08 | 插件依赖隔离 CI | missing | 除 `ah-app` 外,生产 `[dependencies]` 禁止依赖其他 `ah-plugins-*`;dev-dependencies 允许 |
| P1-09 | 卸载与热替换 | partial | 服务、监听器、后台任务、子进程、socket 可逆释放;provider A 可替换为 B |
| P1-10 | 序列化契约版本化 | partial | session、controller snapshot、workflow checkpoint、外部协议有 envelope/version 和迁移策略 |

## P2 日常 Agent 工作负载

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P2-01 | 常用工具对等 | partial | edit、glob、grep、todo、memory、cron 的权限、错误和结构化输出通过 differential |
| P2-02 | rails 与安全策略 | partial | planning/completion/retry/approval、tiered policy 和 overrides 完整接线 |
| P2-03 | context engine | partial | round/dialogue compression、session memory、prompt attachment window mutator 对等 |
| P2-04 | subagents | partial | Python 具体构建器、白名单、上下文继承、取消和结果聚合通过 E2E |
| P2-05 | multi-agent/messager | partial | handoff、订阅拓扑、跨进程 transport、inbox/watch 和并发 governor 可运行 |
| P2-06 | retrieval/memory 生产后端 | partial | 至少一套真实 embedding、reranker、vector store 和 external memory 进入 production E2E |

## P3 演进与外围能力

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P3-01 | agent_evolving LLM 闭环 | partial | judge、experience、optimizer、updater、checkpoint 和 trajectory 持久化完整运行 |
| P3-02 | RSI 主编排 | partial | updater、orchestrator、dataset generator、member optimizer、experience store 和 ledger E2E |
| P3-03 | 外部基础设施 | partial | Redis/Pulsar/ES/GaussDB/Milvus、远程 sandbox、OTel SDK 按声明逐项集成验证 |
| P3-04 | vendor-specific provider | missing | DashScope/Aliyun 等 embedding/rerank/model 适配具备协议、限流、重试和凭据测试 |

## 里程碑

| 里程碑 | 范围 | 可声明结果 |
| --- | --- | --- |
| M1 可信第一阶段 | P0 全部 | 架构和生产组合有可重复验收,进度数字可信 |
| M2 核心 Agent 可替代 | P1 全部 | application/agent-loop/controller/session/workflow 主链通过 Python 差分 |
| M3 日常工作负载可替代 | P2 全部 | coding agent、subagent、团队、检索记忆具备生产实用性 |
| M4 完整迁移 | P3 全部 | 演进、RSI、外部基础设施和厂商能力进入最终验收 |

当前执行顺序:`P0-01 -> P0-02 -> P0-03 -> P0-04 -> P0-05 -> P1-01`。
