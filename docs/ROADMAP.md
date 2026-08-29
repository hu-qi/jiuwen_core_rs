# 当前路线图

> 审计基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`;当前 HEAD:`b455702`(此后仅
> 构建修复 `4c47628`、clippy 门禁修复 `3b57a03`、文档对齐 `b455702`,无功能面变化,审计百分比仍有效)。
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
| P0-01 | 建立真实 Python/Rust differential runner | partial(`differential/`) | 同一语言中立 fixture 分别驱动 Python 与 Rust;CI 比较输出、错误、状态、日志、恢复、取消和超时。MVP 已落地:隔离加载的 Python runner(`run_python.py`)+ Rust reference(`differential.rs` 复用 `settle()`)+ `compare.py`(known_divergence 报告/未标记差异失败)+ CI job;stop_condition 4/4、messager_inprocess 2/2 一致,已记录 1 个真实差异(messager 进程全局总线 vs Rust 每实例总线)。首批 application/agent-loop/session/controller/workflow/tools 六 seam 仍待接入 |
| P0-02 | production profile 静态组合验证 | done(`ah-app/tests/static_composition.rs`) | profile 每个插件可由 catalog 解析;provides/inject 无缺失、重复和环。dev/prod 双 profile 静态校验通过(镜像 mount_all 语义、不触发 apply);发现并修复真实 bug:prod 缺 `ah-plugins-model-backup`(agent-loop inject MODEL_BACKUP)→ 已补入 prod.toml |
| P0-03 | production boot smoke | missing | 无 mock,使用本地协议 fixture 和临时持久化目录完成 boot 与一次 `ApplicationRuntime::invoke` |
| P0-04 | `mount_all` 失败原子性验证 | partial | 后续插件 apply 失败后,此前服务和事件监听器全部回滚,Context 回到调用前状态 |
| P0-05 | 统一审计数据源 | partial | done/partial/missing、域汇总和百分比由结构化数据生成,不再手工累计 |
| P0-06 | 文档与 CI 事实对齐 | done(`b455702`) | README、testing、development、审计和技术目录不再引用过期数字或不存在的门禁 |

首批 differential 范围:application、agent-loop、session、controller、workflow、tools。

## P1 核心 Agent 主链与插件生命周期

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P1-01 | 宿主只消费 `dyn AgentLoopRuntime` | done(`4c47628`) | `ah-app`/CLI 不解析具体 `AgentLoop`;替换 provider 无需修改宿主 |
| P1-02 | 结构化运行错误和真实统计 | done(`AgentResult` 结构化) | 取消/中断/超时不依赖错误字符串;AgentResult 返回真实 iterations、tool calls 和终止原因。`AgentLoopRuntime` 三个方法均返回 `AgentResult`(state + `failure: AgentFailure` + iterations + tool_calls + answer/error);application 已删除错误字符串 contains 判定,直接消费结构化状态 |
| P1-03 | timeout/cancel/interrupt 全链传播 | done(执行中中止,差分待首批六 seam 接入) | 可中断阻塞中的模型、流式和工具调用;事件顺序与恢复行为通过 differential。`race_control` 在模型调用/工具执行期间轮询控制状态与截止时间(25ms),命中即中止在途 future;run 级截止时间约束 backup 链全 provider(跨 provider timeout)。differential 覆盖待 P0-01 六 seam 接入后验证 |
| P1-04 | session/checkpoint 完整恢复 | partial | 崩溃、部分 tool call、fork/restore、幂等、版本兼容和并发访问均有契约测试 |
| P1-05 | application 完整绑定 | partial | 结构化 command、LLM intent、memory/invoke rails 和请求级配置与 Python 公开行为对等 |
| P1-06 | controller 完整行为 | partial | LLM intent、状态机、父子任务、并发调度和 snapshot 迁移通过 differential |
| P1-07 | workflow 流式执行 | partial | STREAM/TRANSFORM/COLLECT、增量工具结果、中断和 checkpoint 续跑完整接线 |
| P1-08 | 插件依赖隔离 CI | done(`ah-app/tests/plugin_isolation.rs`) | 除 `ah-app` 外,生产 `[dependencies]` 禁止依赖其他 `ah-plugins-*`;dev-dependencies 允许。另断言插件生产依赖内部 crate 只允许 `ah-hub`/`ah-contracts`;当前全 workspace 零违规 |
| P1-09 | 卸载与热替换 | partial | 服务、监听器、后台任务、子进程、socket 可逆释放;provider A 可替换为 B |
| P1-10 | 序列化契约版本化 | partial | session、controller snapshot、workflow checkpoint、外部协议有 envelope/version 和迁移策略 |

## P2 日常 Agent 工作负载

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P2-01 | 常用工具对等 | partial(5 工具已实现,差分待 P0-01 六 seam 接入) | edit、glob、grep、todo、memory、cron 的权限、错误和结构化输出通过 differential。edit/glob/grep 已入 ah-plugins-sysop(替换/递归匹配/正则搜索,错误显式、输出结构化,写轴走 pre-execute rails);todo(会话隔离 JSON 持久化 add/update/remove/list)与 cron(cron 五字段子集解析 + next_run + add/list/remove/toggle,文件持久化)已入新 crate ah-plugins-common-tools;memory 工具(remember/recall/forget)既有。differential 覆盖待 P0-01 首批六 seam 接入后补 |
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

当前执行顺序:`P0-01(已完成,见下)-> P0-02(已完成)-> P0-03 -> P0-04 -> P0-05 -> P1-01(已完成,`4c47628`)`。

## 近期工作包(按优先级,2026-08 现状)

> 由 parity-audit(gap 明细)+ ROADMAP 完成标准归纳;每个工作包开工时按「完成口径」记录
> 实现位置、聚焦测试、production 验证与 differential 状态。

| 优先级 | 工作包 | 说明与证据 |
| ---: | --- | --- |
| 1 | P0-01 Python/Rust differential runner(MVP) | ✅ 已落地(`8f735a4`):`differential/`(run_python.py/compare.py/run.sh/README)+ CI job;stop_condition 4/4、messager_inprocess 2/2 一致,1 个已知差异已记录;首批六 seam 待接入 |
| 2 | P0-02 production 静态组合验证 → CI | ✅ 已落地:`ah-app/tests/static_composition.rs`(catalog 解析 + 无重复/缺失 provider + 无环),dev/prod 双 profile 通过;修复 prod 缺 `ah-plugins-model-backup` 的真实 bug |
| 3 | P1-08 插件依赖隔离 CI | 机械检查:除 `ah-app` 外,生产 `[dependencies]` 禁止引用其他 `ah-plugins-*` |
| 4 | P1-02/03 agent-loop 结构化错误与执行中取消 | ✅ 已落地:`AgentLoopRuntime` 返回结构化 `AgentResult`(state/failure/iterations/tool_calls,application 不再解析错误字符串);`race_control` 中止在途模型/工具调用 + run 级截止时间约束 backup 链;4 个新聚焦测试(timeout/interrupt 中止在途调用、精确统计) |
| 5 | P2-01 确定性工具补齐 | ✅ 主体已落地:edit/glob/grep(ah-plugins-sysop)+ todo/cron(新 crate ah-plugins-common-tools,含 cron 五字段解析与 next_run);memory 既有;差分验证待 P0-01 六 seam 接入 |
| 6 | P1-07 workflow 流式执行 | STREAM/TRANSFORM/COLLECT、增量工具结果、中断与 checkpoint 续跑完整接线 |
| 7 | P1-05/06 application/controller LLM 意图 | 结构化 command 载荷 + LLM intent 识别替换关键字匹配 |
| 8 | P0-04 / P0-05 | `mount_all` 失败原子性验证;统一审计数据源(百分比改由结构化账本生成) |
| 9 | P2-02 rails 长尾 | planning/completion/retry/memory/skills/interrupt/context_engineer 等 LLM 型 rail |
| 10 | P2-03 context round/dialogue 压缩 | round/dialogue 压缩、会话记忆管理器、prompt attachment window mutator |
| 11 | P2-04/05/06 | subagents browser/mobile、messager pyzmq 跨进程 + handoff、retrieval embedding/vector store 生产后端 |
| 12 | P3-01/02 | evolving LLM 闭环(judge/experience/optimizer/updater);RSI 主编排(updater missing、orchestrator、dataset generator) |
| 13 | P3-03/04 | 外部基础设施(Pulsar/ES/GaussDB/Milvus/远程沙箱/OTel SDK);vendor-specific provider(missing) |
