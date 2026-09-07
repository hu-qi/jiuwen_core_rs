# 当前路线图

> 审计快照基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`;当前实现代码 revision:`f952a29`;后续审计与文档提交不改变该实现基线。
> `audit/ledger.json` 是工作包状态、域汇总和状态百分比的唯一结构化来源;`docs/generated/audit-summary.md` 由 `audit-ledger` 生成。本文保留验收标准和执行顺序,不再手工累计状态数字。
> HEAD 在旧快照之后新增了多个插件 crate;阶段一已将 10 个新增插件加入 Cargo workspace members,阶段二已将它们接入 `ah-app::plugin_catalog` 与 dev/prod Profile,并完成 targeted mount/resolve/invoke/unmount 验证;源码和 targeted 集成测试通过不等于 production 能力完成。
> 产品实现、默认 Rust 回归和生产 Profile 不依赖 Python。`rails-differential` CI job 仅对固定 agent-core 参考提交执行四个可同层比较的 LLM Rails seam;其余 Python differential 脚本仍是可选历史审计。当前执行顺序以本文件为准;能力明细区分 implementation、production verification 和历史规格差异。

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

首批 Rust contract 范围:`session`、`tools`、`controller`、`agent-loop`、`workflow`、`application`;验收不依赖 Python。

## P1 核心 Agent 主链与插件生命周期

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P1-01 | 宿主只消费 `dyn AgentLoopRuntime` | done(`4c47628`) | `ah-app`/CLI 不解析具体 `AgentLoop`;替换 provider 无需修改宿主 |
| P1-02 | 结构化运行错误和真实统计 | done(`AgentResult` 结构化) | 取消/中断/超时不依赖错误字符串;AgentResult 返回真实 iterations、tool calls 和终止原因。`AgentLoopRuntime` 三个方法均返回 `AgentResult`(state + `failure: AgentFailure` + iterations + tool_calls + answer/error);application 已删除错误字符串 contains 判定,直接消费结构化状态 |
| P1-03 | timeout/cancel/interrupt 全链传播 | done(执行中中止已实现) | 可中断阻塞中的模型、流式和工具调用;`race_control` 在模型调用/工具执行期间轮询控制状态与截止时间(25ms),命中即中止在途 future;run 级截止时间约束 backup 链全 provider。生产 provider 全覆盖仍需单独验证 |
| P1-04 | session/checkpoint 完整恢复 | done(`ah-plugins-session-log` + `ah-plugins-agent-loop`) | JSONL 崩溃末行修复、完整行损坏拒绝、checkpoint/fork/restore 原子替换、Unix 跨进程追加锁与 seq 连续性;工具调用 claim 在锁内重读并保证跨进程单 owner;流式模型/工具 delta 在下游发送前落盘;恢复只自动执行声明 `idempotent() = true` 且消费稳定 call ID 的工具,非幂等工具写入 `status=unknown` 并返回 `ToolRecoveryRequired`;测试覆盖成功恢复、拒绝重试和后续不重复执行 |
| P1-05 | application 完整绑定 | done(`production-p1`) | 结构化 command、LLM intent、memory/invoke rails、请求级配置和 workflow stream 已接入 Rust `ApplicationRuntime`;prod.toml 真实插件组合通过 Redis + 本地确定性 OpenAI-compatible fixture 完成 invoke/stream E2E |
| P1-06 | controller 完整行为 | done(`production-p1`) | LLM intent、状态机、父子任务、多条件过滤、跨 session 并发调度和 snapshot 迁移已接入 Rust `Controller`;production-p1 覆盖 create/register/run_pending/restart restore |
| P1-07 | workflow 流式执行 | done(`production-p1`) | `WorkflowStreamSink` 按序发出 `workflow_delta`、`workflow_node`、`workflow_resume`、`workflow_final`;checkpoint 版本头、fsync、损坏记录拒绝和节点恢复已测试;production-p1 覆盖真实 prod 组合的 stream |
| P1-08 | 插件依赖隔离 CI | done(`ah-app/tests/plugin_isolation.rs`) | 除 `ah-app` 外,生产 `[dependencies]` 禁止依赖其他 `ah-plugins-*`;dev-dependencies 允许。另断言插件生产依赖内部 crate 只允许 `ah-hub`/`ah-contracts`;当前全 workspace 零违规 |
| P1-09 | 正常关闭资源释放 | done(`ah-plugins-external` Drop/kill_on_drop + MCP 有界 shutdown) | 应用关闭与 mount 失败时撤销服务/监听器,终止后台 task、子进程、socket 和 watcher;运行中 provider 热替换不属于当前产品范围 |
| P1-10 | 序列化契约版本化 | done(`ah-plugins-transport` JSON-RPC 2.0 policy) | session、controller snapshot、workflow checkpoint 使用 envelope/version 与 legacy 读取;外部 JSON-RPC 固定 2.0,客户端校验响应 version/id,服务端拒绝未知版本 |
> 产品范围明确不支持运行中热插拔;provider replacement、动态 ABI 和运行中重新装载不作为当前完成门禁。
> 任务重叠: P1-03 负责跨模型/工具/工作流的统一取消传播;P1-10 负责 checkpoint envelope/version 迁移;P1-05 负责 ApplicationRuntime 对 workflow stream 的宿主级暴露;P2-02 负责 rails 层 interrupt/approval/retry。P1-07 只拥有 workflow stream 执行、chunk 顺序、背压和节点 checkpoint 行为,不重复实现这些公共能力。

## P2 日常 Agent 工作负载

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P2-01 | 常用工具对等 | done(`ah-plugins-sysop` + `ah-plugins-common-tools` + `ah-plugins-memory` + `ah-plugins-rails`) | edit、glob、grep、todo、memory、cron 的权限、错误和结构化输出由真实 Rust provider 提供;LocalFsProvider 拒绝 symlink 写入逃逸,PathGuard 覆盖所有带 path 的确定性工具;Rust-only fixture、挂载 golden、生产 profile smoke 均通过;Python 侧 API/宿主依赖不同,按 `not-comparable` 记录,不伪造 parity |
| P2-02 | Rails 与安全策略 | done | `ah-plugins-rails` 已实现 planning/completion/retry/approval 与安全 rail; `ah-plugins-security/src/tiered_policy.rs` 已实现参数规则、`approval_overrides`、整工具/默认优先级、shell AST 安全下限和 network URL/query 匹配; `PermissionApprovalProvider` 已提供 AllowOnce/AllowAlways/Deny HITL seam,production `security-policy` bundle 已接入 `prod.toml`; Python/Rust differential 对 agent-core@aeb88cd8 已验证 Rails 四类 26 matched、GoalManager lifecycle 2 matched、PromptAttachment CRUD/filter/expiry 1 matched、runtime model switching 2 matched、task lifecycle 1 matched、subagents 3 matched、cancellation callback 4 matched,总计 39 matched、0 known divergence、0 mismatch |
| P2-03 | context engine | done(`ah-plugins-context` + focused tests) | round/dialogue compression 按完整 tool round 保留、session memory 摘要缓存、prompt attachment window mutator;dev boot、static composition 与 workspace regression 通过 |
| P2-04 | subagents | done(`ah-plugins-subagent` + `ah-plugins-subagents` + `ah-plugins-mcp`) | Rust-only browser/mobile path includes Playwright MCP stdio cwd/env/capabilities, standard MCP image forwarding, request-scoped Android serial enforcement, health-before-action, dynamic scroll resolution, key events, screenshots, OpenAI/Anthropic image messages and session-log projection; typed browser factory injects compact probe/card/batch/custom-action helpers and prompts enforce evidence/probe ordering; Python browser probe lifecycle 与 DeviceLifecycleRail/coordinate action application flow fixture 2 matched、0 known divergence、0 mismatch; deterministic `com.example.todo` flow and real Android AVD `emulator-5556` Settings flow (health → screenshot → grounded tap → fresh screenshot) passed |
| P2-05 | multi-agent/messager | done(`ah-plugins-messager` + `ah-plugins-teams` + `ah-plugins-external`) | `ah-plugins-messager` 已实现 libzmq ROUTER/DEALER P2P(known peer 路由、recipient 标识、ACK、超时与显式错误)、PUB/SUB 广播(绑定 proxy、订阅/取消订阅、sender_id 盖章、停止清理);InMemoryTeamRuntime 与 SqliteTeamRuntime 在 messager seam 注册时使用 team message topic,任务状态同时发布 legacy task topic,无 messager 时保留 queue 持久化路径;任务完成现在要求 `in_progress` 状态并保留 result payload;ExternalTeamClient watch 已订阅 session-scoped MESSAGE/TASK 与 legacy message/task topics,事件触发 inbox 重取,空 inbox 不回调,取消时自动取消订阅;external-format 与 external-client fetch 对 agent-core@aeb88cd8 的消息渲染、任务看板、mark-read、watch callback/cancellation differential 已 7 matched、0 known divergence、0 mismatch;Python full team handoff/task mutation lifecycle 已完成: task create/claim/update/dependency/review 对 agent-core@aeb88cd8 1 case matched, 0 known divergence, 0 mismatch; Rust InMemory/SQLite 两种任务板均通过完整生命周期测试 |
| P2-06 | retrieval/memory 生产后端 | done(`ah-plugins-retrieval` + `ah-plugins-rerank` + `ah-plugins-memory` + Redis store/queue) | production profile 使用真实 Redis KV/vector/memory 后端;OpenAI-compatible embedding 真实 HTTPS E2E;DashScope 原生 embedding 与 query-aware rerank 通过 production profile 协议 E2E;向量与外部 memory 重启恢复、删除清理、配置优先级与显式错误均已验证;真实厂商凭据专项仍由 P3-04 负责 |

## P3 演进与外围能力

| ID | 任务 | 当前状态 | 完成标准 |
| --- | --- | --- | --- |
| P3-01 | agent_evolving LLM 闭环 | done | `ah-plugins-evolving` 已完成真实 session→trajectory checkpoint→LLM judge→本地/LLM optimizer→Experience JSONL 闭环,包含在线编排、更新预览、生命周期 staging、技能与 script assets 原子落盘、外部 BaseKVStore、AgentStep 持久化事件和 production profile E2E。 |
| P3-02 | RSI 主编排 | done | `ah-plugins-rsi` 已完成 LLM 数据集生成、真实 subagent 执行、evolving 评估/提示精化、JSONL/外部 checkpoint 续跑、ledger 检索、member optimizer 与 updater 应用路径;Redis checkpoint resume 已通过 production profile E2E。 |
| P3-03 | 外部基础设施 | done | 已完成 Redis/PostgreSQL/GaussDB wire alias、Elasticsearch REST、Pulsar REST、Milvus REST v2、远程 sandbox fail-closed、OTLP/JSON telemetry;实现和本地协议 contract tests 已通过,现场服务 E2E 记录在外部环境验收项。 |
| P3-04 | vendor-specific provider | done | 已完成 DashScope 原生 embedding/rerank、OpenAI-compatible Qwen、Anthropic provider、credentials/env 接线、重试、响应校验和串行化;OpenAI production HTTPS 已通过,DashScope/Anthropic 有协议测试,live vendor smoke 需凭据。 |

## 里程碑

| 里程碑 | 范围 | 可声明结果 |
| --- | --- | --- |
| M1 可信第一阶段 | P0 全部 | 架构和生产组合有可重复验收,进度数字可信 |
| M2 核心 Agent 可替代 | P1 全部 | application/agent-loop/controller/session/workflow 主链通过 Python 差分 |
| M3 日常工作负载可替代 | P2 全部 | coding agent、subagent、团队、检索记忆具备生产实用性 |
| M4 完整迁移 | P3 全部 | 演进、RSI、外部基础设施和厂商能力进入最终验收 |

当前 P0-P3 工作包均已完成。下一步是最终验收收尾:完成可用外部服务的现场 E2E、无超时的 workspace 全量门禁、真实 vendor 凭据 smoke,并维护账本与生成文档的一致性。

## 近期工作包(按优先级,2026-09 当前复核)

> 由 parity-audit(gap 明细)+ ROADMAP 完成标准归纳;每个工作包开工时按「完成口径」记录
> 实现位置、聚焦测试、production 验证与 differential 状态。

| 优先级 | 工作包 | 说明与证据 |
| ---: | --- | --- |
| 1 | P0-01 Rust-only contract runner | ✅ 已落地:`ah-app/tests/rust_contract.rs` + versioned fixtures;不依赖 Python,作为默认回归门禁 |
| 2 | P0-02 production 静态组合验证 → CI | ✅ 已落地:`ah-app/tests/static_composition.rs`(catalog 解析 + 无重复/缺失 provider + 无环),dev/prod 双 profile 通过;修复 prod 缺 `ah-plugins-model-backup` 的真实 bug |
| 3 | P1-08 插件依赖隔离 CI | ✅ 已落地(`6960700`):`ah-app/tests/plugin_isolation.rs`(生产依赖禁引插件 + 插件只依赖 hub/contracts),全 workspace 零违规 |
| 4 | P1-02/03 agent-loop 结构化错误与执行中取消 | ✅ 已落地:`AgentLoopRuntime` 返回结构化 `AgentResult`(state/failure/iterations/tool_calls,application 不再解析错误字符串);`race_control` 中止在途模型/工具调用 + run 级截止时间约束 backup 链;4 个新聚焦测试(timeout/interrupt 中止在途调用、精确统计) |
| 5 | P2-01 确定性工具补齐 | ✅ 已完成:`ah-plugins-sysop` 的 edit/glob/grep + `ah-plugins-common-tools` 的 todo/cron + `ah-plugins-memory` 的 remember/recall/forget;`fixtures/tools.json` 由 Rust contract runner 执行,覆盖结构化输出、权限拒绝、正则/替换校验、session 隔离持久化、cron schedule、memory 恢复与错误分类;`tools_golden` 与 focused tests 通过 |
| 6 | P1-07 workflow 流式执行 | ✅ Rust stream 主链与组件能力已落地:`WorkflowStreamSink`、`WorkflowEngine::stream`、`stream_checkpointed`、`WorkflowComponentRegistry`、`NodeKind::Component`;LLM 增量、节点/恢复/最终 chunk 按序发送,producer/consumer 可并发且不持有 receiver 锁,取消时关闭 sink,组件 invoke/stream/collect/transform 端到端测试通过;`ah-plugins-workflow` 17/17、`ah-plugins-stream` 9/9、`ah-app` workflow contract fixture 通过;Python ActorManager 多 producer/source-group 语义、节点级中断恢复、workflow differential 仍为 partial |
| 7 | P1-05/06 application/controller references | ✅ Rust reference traces、stream、scheduler 和 checkpoint 版本已落地;Python 仅为历史规格,不进入 Rust-only 验收 |
| 8 | P0-05 | ✅ 已落地:`audit/ledger.json` + `ah-app audit-ledger`;生成整体/域状态汇总、状态百分比、未完成工作包和逐包证据;CI freshness gate 已接入 |
| 9 | P0-01 Rust-only contract fixture 扩展 | ✅ 已落地:`ah-app/tests/rust_contract.rs` + versioned fixtures;默认 CI 不调用 Python |
| 10 | P2-02 rails 长尾 | ✅ 已完成:`ah-plugins-rails` planning/completion/retry/approval 与安全 rail;`ah-plugins-security` tiered policy/overrides、shell/network 分层匹配、HITL `PermissionApprovalProvider` 和 fail-closed pre-execute 已接线;GoalManager、Prompt Attachment、runtime model switching、cancellation callback checkpoints、task lifecycle、subagent 与 cancellation callback differential 已验证;P2-02 Rust 核心回归 189 passed、Python differential 39 matched、生产 profile smoke 通过 |
| 11 | P2-03 context round/dialogue 压缩 | ✅ 已落地:`ah-plugins-context` 按完整 user→assistant final round 保留窗口,不切断 tool-call/tool-result;SessionMemoryManager 缓存相同 session 前缀摘要;ContextEngine 消费 prompt attachment window mutator;context 7/7、AgentLoop 21/21、app contract 15/15、workspace 1376 通过 |
| 12 | P2-04/05/06 | subagents browser/mobile、messager pyzmq 跨进程 + handoff、retrieval embedding/vector store 生产后端 |
| 13 | P3-01/02 | ✅ 已完成:生产 profile 真实 OpenAI-compatible LLM 闭环;AgentStep 完成事件持久化;evolving judge/experience/optimizer/updater;RSI dataset generation、真实 subagent 执行、evolving 评估/精化、Redis checkpoint resume |
| 14 | P3-03/04 | ✅ 已完成: Pulsar REST proxy、Elasticsearch REST、GaussDB PostgreSQL-wire alias、Milvus REST v2、远程 sandbox fail-closed 与 OTLP/JSON;真实配置 vendor HTTPS smoke、DashScope/Anthropic 协议测试。Pulsar/ES/GaussDB/Milvus/远程沙箱的现场部署 E2E 仍需对应服务环境 |
