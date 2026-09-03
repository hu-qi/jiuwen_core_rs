# 当前路线图

> 审计快照基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`;当前代码 HEAD:`14b0c16`。
> `audit/ledger.json` 是工作包状态、域汇总和状态百分比的唯一结构化来源;`docs/generated/audit-summary.md` 由 `audit-ledger` 生成。本文保留验收标准和执行顺序,不再手工累计状态数字。
> HEAD 在旧快照之后新增了多个插件 crate;阶段一已将 10 个新增插件加入 Cargo workspace members,阶段二已将它们接入 `ah-app::plugin_catalog` 与 dev/prod Profile,并完成 targeted mount/resolve/invoke/unmount 验证;源码和 targeted 集成测试通过不等于 production 能力完成。
> 当前 Python differential 仅覆盖 `stop_condition`、`messager_inprocess` 两个 seam,共 6 个匹配 case,另有 1 个已知差异。Rust-only contract runner 已接入 `session`、`tools`、`controller`、`agent-loop`、`workflow`、`application`;本轮新增 application/controller Rust reference traces,但 agent-core 没有与 Rust `ApplicationRuntime` 等价的 Python runtime,因此不能伪造 Python parity 结论。
> 当前执行顺序以本文件为准;能力明细必须同时区分 implementation、workspace/runtime integration、production verification 和 Python parity。

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
| P0-01 | 建立 Rust-only contract fixture runner | done(`ah-app/tests/rust_contract.rs`;6 test functions) | `fixtures/{session,tools,controller,agent_loop,workflow,application}.json` 使用 schema version=1;Rust runner 比较规范化状态、顺序、错误分类和恢复结果,不导入 Python;Python differential 仅在可处于同一抽象层时单独记录,不得作为 Rust runtime prerequisite |
| P0-02 | production profile 静态组合验证 | done(`ah-app/tests/static_composition.rs`)+阶段二 runtime 接线 | profile 每个插件可由 catalog 解析;provides/inject 无缺失、重复和环。dev/prod 双 profile 静态校验、10 个新增插件 catalog/Profile 接线及 targeted mount/resolve/invoke/unmount 验证通过;不替代无 mock production boot |
| P0-03 | production boot smoke | done(`crates/ah-app/tests/production_boot.rs`) | opt-in smoke 使用显式 `AH_ENV_FILE` 防止陈旧环境变量覆盖;真实 Redis KV/queue/checkpointer、OpenAI chat/stream 和 `ApplicationRuntime::invoke` 均通过;测试命令见 `production_boot.rs` |
| P0-04 | `mount_all` 失败原子性验证 | done(`ah-hub` context unit test) | 后续插件 apply 失败后,此前服务和事件监听器全部回滚,Context 回到调用前状态 |
| P0-05 | 统一审计数据源 | done(`audit/ledger.json` + `ah-app audit-ledger`) | `audit/ledger.json` 记录每个工作包的状态、实现位置、验证命令、production 和 differential 证据;`audit-ledger` 生成总览、域汇总、状态百分比和未完成清单;CI 检查生成摘要无漂移 |
| P0-06 | 文档与 CI 事实对齐 | done(current working tree review) | README、testing、审计、能力映射、配置和依赖目录已明确当前 HEAD 与历史快照边界;提交后需保留当前基线 |

首批 Rust contract 范围:`session`、`tools`、`controller`、`agent-loop`、`workflow`、`application`;Python differential 仍保留这些领域作为可选行为审计范围。

## P1 核心 Agent 主链与插件生命周期

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P1-01 | 宿主只消费 `dyn AgentLoopRuntime` | done(`4c47628`) | `ah-app`/CLI 不解析具体 `AgentLoop`;替换 provider 无需修改宿主 |
| P1-02 | 结构化运行错误和真实统计 | done(`AgentResult` 结构化) | 取消/中断/超时不依赖错误字符串;AgentResult 返回真实 iterations、tool calls 和终止原因。`AgentLoopRuntime` 三个方法均返回 `AgentResult`(state + `failure: AgentFailure` + iterations + tool_calls + answer/error);application 已删除错误字符串 contains 判定,直接消费结构化状态 |
| P1-03 | timeout/cancel/interrupt 全链传播 | done(执行中中止,差分待首批六 seam 接入) | 可中断阻塞中的模型、流式和工具调用;事件顺序与恢复行为通过 differential。`race_control` 在模型调用/工具执行期间轮询控制状态与截止时间(25ms),命中即中止在途 future;run 级截止时间约束 backup 链全 provider(跨 provider timeout)。differential 覆盖待 P0-01 六 seam 接入后验证 |
| P1-04 | session/checkpoint 完整恢复 | done(`ah-plugins-session-log` + `ah-plugins-agent-loop`) | JSONL 崩溃末行修复、完整行损坏拒绝、checkpoint/fork/restore 原子替换、Unix 跨进程追加锁与 seq 连续性;工具调用 claim 在锁内重读并保证跨进程单 owner;流式模型/工具 delta 在下游发送前落盘;恢复只自动执行声明 `idempotent() = true` 且消费稳定 call ID 的工具,非幂等工具写入 `status=unknown` 并返回 `ToolRecoveryRequired`;测试覆盖成功恢复、拒绝重试和后续不重复执行 |
| P1-05 | application 完整绑定 | partial(核心绑定与 Rust reference 已实现) | 结构化 command、LLM intent、memory/invoke rails 和请求级配置与 Python 公开行为对等;当前 Rust 已接入 command/task create、模型 intent、按 user_id 隔离的 memory 上下文读写及 model/temperature/timeout 覆盖;`references/application.json` 覆盖 agent/workflow/invalid request traces,Python parity 因缺少等价 runtime 仍未验证 |
| P1-06 | controller 完整行为 | partial(核心行为与 Rust reference 已实现) | LLM intent、状态机、父子任务、并发调度和 snapshot 迁移通过 differential;当前 Rust 已接入严格 JSON intent、session 工作任务原子预约、稳定优先级排序、多条件/递归子任务过滤、跨 session 并发调度和 v2 snapshot 读取;`references/controller.json` 覆盖 lifecycle/illegal/keyword traces,Python parity 仍未验证 |
| P1-07 | workflow 流式执行 | partial(Rust stream/取消/checkpoint stream/组件能力已接通,严格 Python parity 仍待) | `WorkflowStreamSink` 提供背压接收端;workflow 按序发出 `workflow_delta`、`workflow_node`、`workflow_resume`、`workflow_final`;LLM 流并发消费避免队列背压死锁;stream manager 支持 END_FRAME 和协作式 cancel;`stream_checkpointed` 支持节点输出续跑;`WorkflowComponentRegistry` + `NodeKind::Component` 已实现 invoke/stream/collect/transform。Python ActorManager 的多 producer stream-edge/source-group、节点级中断恢复和 differential 仍待后续验收 |
| P1-08 | 插件依赖隔离 CI | done(`ah-app/tests/plugin_isolation.rs`) | 除 `ah-app` 外,生产 `[dependencies]` 禁止依赖其他 `ah-plugins-*`;dev-dependencies 允许。另断言插件生产依赖内部 crate 只允许 `ah-hub`/`ah-contracts`;当前全 workspace 零违规 |
| P1-09 | 正常关闭资源释放 | done(`ah-plugins-external` Drop/kill_on_drop + MCP 有界 shutdown) | 应用关闭与 mount 失败时撤销服务/监听器,终止后台 task、子进程、socket 和 watcher;运行中 provider 热替换不属于当前产品范围 |
| P1-10 | 序列化契约版本化 | partial(session + controller snapshot 已有版本读取) | session、controller snapshot、workflow checkpoint、外部协议有 envelope/version 和迁移策略 |
> 产品范围明确不支持运行中热插拔;provider replacement、动态 ABI 和运行中重新装载不作为当前完成门禁。
> 任务重叠: P1-03 负责跨模型/工具/工作流的统一取消传播;P1-10 负责 checkpoint envelope/version 迁移;P1-05 负责 ApplicationRuntime 对 workflow stream 的宿主级暴露;P2-02 负责 rails 层 interrupt/approval/retry。P1-07 只拥有 workflow stream 执行、chunk 顺序、背压和节点 checkpoint 行为,不重复实现这些公共能力。

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

当前执行顺序:`P0-01(Rust-only contract runner 已接入 session/tools/controller/agent-loop/workflow/application;Python differential 仍为辅助)-> P0-02(静态组合+runtime 接线)-> P0-03(done,真实 Redis/OpenAI opt-in boot smoke)-> P0-04(done,失败原子性已测)-> P0-05(done,结构化审计账本已接入)-> P1-01(实现已完成,宿主已 seam 化)`。

## 近期工作包(按优先级,2026-09 当前复核)

> 由 parity-audit(gap 明细)+ ROADMAP 完成标准归纳;每个工作包开工时按「完成口径」记录
> 实现位置、聚焦测试、production 验证与 differential 状态。

| 优先级 | 工作包 | 说明与证据 |
| ---: | --- | --- |
| 1 | P0-01 Python/Rust differential runner(MVP) | ✅ 已落地(`8f735a4`):`differential/`(run_python.py/compare.py/run.sh/README)+ CI job;stop_condition 4/4、messager_inprocess 2/2 一致,1 个已知差异已记录;首批六 seam 待接入 |
| 2 | P0-02 production 静态组合验证 → CI | ✅ 已落地:`ah-app/tests/static_composition.rs`(catalog 解析 + 无重复/缺失 provider + 无环),dev/prod 双 profile 通过;修复 prod 缺 `ah-plugins-model-backup` 的真实 bug |
| 3 | P1-08 插件依赖隔离 CI | ✅ 已落地(`6960700`):`ah-app/tests/plugin_isolation.rs`(生产依赖禁引插件 + 插件只依赖 hub/contracts),全 workspace 零违规 |
| 4 | P1-02/03 agent-loop 结构化错误与执行中取消 | ✅ 已落地:`AgentLoopRuntime` 返回结构化 `AgentResult`(state/failure/iterations/tool_calls,application 不再解析错误字符串);`race_control` 中止在途模型/工具调用 + run 级截止时间约束 backup 链;4 个新聚焦测试(timeout/interrupt 中止在途调用、精确统计) |
| 5 | P2-01 确定性工具补齐 | ✅ 主体已落地:edit/glob/grep(ah-plugins-sysop)+ todo/cron(新 crate ah-plugins-common-tools,含 cron 五字段解析与 next_run);memory 既有;差分验证待 P0-01 六 seam 接入 |
| 6 | P1-07 workflow 流式执行 | ✅ Rust stream 主链与组件能力已落地:`WorkflowStreamSink`、`WorkflowEngine::stream`、`stream_checkpointed`、`WorkflowComponentRegistry`、`NodeKind::Component`;LLM 增量、节点/恢复/最终 chunk 按序发送,producer/consumer 可并发且不持有 receiver 锁,取消时关闭 sink,组件 invoke/stream/collect/transform 端到端测试通过;`ah-plugins-workflow` 17/17、`ah-plugins-stream` 9/9、`ah-app` workflow contract fixture 通过;Python ActorManager 多 producer/source-group 语义、节点级中断恢复、workflow differential 仍为 partial |
| 7 | P1-05/06 application/controller LLM 意图与行为 references | ✅ Rust reference traces 已落地(`ah-app/tests/differential.rs`,`references/{application,controller}.json`);Python differential 仍仅对现有两个可同层 seam 执行,ApplicationRuntime 缺少 Python 等价实现 |
| 8 | P0-05 | ✅ 已落地:`audit/ledger.json` + `ah-app audit-ledger`;生成整体/域状态汇总、状态百分比、未完成工作包和逐包证据;CI freshness gate 已接入 |
| 9 | P0-01 Rust-only contract fixture 扩展 | ✅ 已落地:`ah-app/tests/rust_contract.rs` + `fixtures/session.json`/`tools.json`/`controller.json`/`agent_loop.json`/`workflow.json`/`application.json`;Python differential 保留为可选辅助审计 |
| 10 | P2-02 rails 长尾 | planning/completion/retry/memory/skills/interrupt/context_engineer 等 LLM 型 rail |
| 11 | P2-03 context round/dialogue 压缩 | round/dialogue 压缩、会话记忆管理器、prompt attachment window mutator |
| 12 | P2-04/05/06 | subagents browser/mobile、messager pyzmq 跨进程 + handoff、retrieval embedding/vector store 生产后端 |
| 13 | P3-01/02 | evolving LLM 闭环(judge/experience/optimizer/updater);RSI 主编排(updater missing、orchestrator、dataset generator) |
| 14 | P3-03/04 | 外部基础设施(Pulsar/ES/GaussDB/Milvus/远程沙箱/OTel SDK);vendor-specific provider(missing) |
