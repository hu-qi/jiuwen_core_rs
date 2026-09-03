# agent-core 未完成插件化功能审计(gap-audit.md)

> 历史审计基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`;当前代码 HEAD:`7404142`。
> 本文件原先以 `b455702` 为最新状态;该基线之后已新增多个插件和功能,原文状态不可直接用于当前 HEAD。
> `partial` 表示已有部分 Rust 实现,但仍缺 runtime integration、production verification 或 Python parity;`missing` 表示当前没有可调用的对应实现。源码存在、单元测试通过或 Rust 自生成 reference 都不能单独升级状态。

## 0. 当前复核结论

- 当前 Cargo workspace 由 `cargo metadata --no-deps` 实际发现 112 个 package;仓库中有 112 个 Cargo manifest。
- 以下 **10 个**新增插件已进入 workspace members,并已接入 `ah-app::plugin_catalog` 与 dev/prod Profile;阶段二 targeted mount/resolve/invoke/unmount 测试通过,但完整 Python 行为和 production boot 仍未完成:`ah-plugins-agentbuilder`、`ah-plugins-a2a`、`ah-plugins-data-loader`、`ah-plugins-dataset-curator`、`ah-plugins-model-allocator`、`ah-plugins-prompt-attachment`、`ah-plugins-interaction-router`、`ah-plugins-inbound-render`、`ah-plugins-external-format`、`ah-plugins-bridge-compose`。
- Agent Loop 当前是基础日志驱动 ReAct;相对 Python `ReActAgent.invoke` 仍缺完整 rails/lifecycle、HITL/workflow interrupt 恢复、steering、并行 tool call、multimodal、stream 和取消清理语义。
- AgentBuilder 当前是关键词设计 + 空参数串行 Workflow DSL,不是 Python 的 LLM builder/executor、session history 和状态机。
- DataLoader 当前提供 JSON 解析、画像、批次规划和 YAML 写入原语,尚无 Python `DataLoader.load` 的目录扫描、文件加载、路径/索引注入和迭代完整入口。
- A2A 当前主要是纯 JSON 适配;Python 的 protobuf 对象、真实 client/server、Task event 和完整 SSE 行为仍未对等。
- `ah-hub::Context::mount_all` 已通过中途 `apply` 失败时服务和事件监听器回滚测试;Effect vector 的自然析构保证此前注册全部撤销。后台资源释放和 provider 热替换仍是 P1-09 缺口。
- 当前 Python/Rust differential 只覆盖 `stop_condition`、`messager_inprocess` 两个 seam,共 6 个 case,另有 1 个已知差异;首批六 Seam 仍未接入。
- 阶段一验证结果:10 个新增插件已加入 workspace;阶段二已接入 catalog/Profile,并通过 1 个 `ah-app` mount/resolve/invoke/unmount 集成测试;`cargo check --workspace --offline` 通过;新增插件单元测试共 117 个 case 全部通过。上述测试只证明 Rust 内部接线和回归稳定,不升级 Python parity 或 production verification。

旧版逐域缺口表仍可作为历史线索,但必须结合 `parity-audit.md` 的重新审计结果使用。

## 1. core(2226 符号 / 796 文件)—— 缺口最大

| Python 子模块 | 状态 | 已落地 | 未完成(缺口) |
| --- | --- | --- | --- |
| application(llm_agent/workflow_agent) | **partial** | agentbuilder 已提供 NL→设计→DSL→workflow 执行;controller/workflow/agent-loop 提供部分运行时能力;agent-control 可供上层发起控制，callback manager 已覆盖生命周期顺序与快照读取 | `llm_agent.py`/`llm_controller.py`/`workflow_agent.py`/`workflow_controller.py`/`workflow_event_handler.py`/`workflow_task_executor.py` 的完整绑定、LLM 澄清/路由、交互恢复、invoke-result/memory rail 仍未对等;已新增 `ah-plugins-application`，通过 SessionManager 按 session_id 路由并持久化 LLM/workflow 请求；控制命令支持 `verb:task-id` 及自然语言 `task-...` 标识提取并经 Controller seam 路由（`routes_natural_language_cancel_intent_to_controller`）；本回合 application 改为仅消费 ah-contracts 的 `AgentLoopRuntime` seam，移除对 agent-loop 具体类型的生产依赖；Controller retry seam 与 `retry:<task_id>` 路由已落地并覆盖 Failed→Submitted、错误清理、持久化和非法错误；`resume:<task_id>` 会在状态恢复后重新调用 `Controller::run_task`；controller 24 项与 application 13 项 focused tests 已通过；agent-loop 失败会追加带 command/state/error 的 System event；`AgentLoopRuntime::card()` 已提供 AgentCard 能力描述，具体 agent-loop 覆盖为 tool-use/interrupt/cancel/timeout/session-recovery；完整 application 绑定仍未对等；timeout 已通过 AgentLoopRuntime contracts seam 进入 ApplicationRequest→AgentLoop 配置，并在轮次边界返回 TimedOut AgentResult；仍未覆盖阻塞中的模型/工具调用 |
| multi_agent(handoff/hierarchical/msgbus) | **partial** | teams/teams-sqlite/teams-workflow、team-message、team-dispatch、team-pool、messager in-process 已落地部分能力 | handoff 团队(container_agent/handoff_orchestrator/handoff_signal/handoff_tool/interrupt)、hierarchical_msgbus(supervisor_agent/p2p_ability_manager)、完整 hierarchical_tools/team_runtime 消息拓扑、跨进程 pyzmq 与 team_card 仍未对等;当前无独立 core-multi-agent crate |
| single_agent | partial | agent-loop(ReAct)已落地;agent-control 提供 interrupt/callback seam;session log 记录取消/中断;skills 文件后端注册、持久化和 subagent/evolving 评估已落地;prompts/builder 部分由 prompt-builder 覆盖 | 可配置 timeout 的完整消费、remote skills、生产 backup provider 配置（catalog seam 与 StaticModelProviderCatalog 已实现，真实多 provider catalog 宿主组装尚缺；Profile 配置已能传入 backup provider_names；retry/timeout 数值已由 app 解析并通过 policy plugin 注入 AgentLoop；dev profile 已声明 model-backup，policy plugin 由 app 按配置动态插入并加入实际挂载列表；helper 已覆盖非法 retry/timeout；完整 app 测试仍受 workspace 编译时限限制；真实 catalog 组装仍缺）、跨 provider cancellation 传播与跨 provider timeout；基础主模型失败恢复且最终错误持久化（AgentLoop 模型与 backup 失败持久化精确测试已通过）、单次 timeout、bounded retry、streaming fallback/retry 和 sink-close 显式失败已完成、rail、kv_cache hooks、application 层完整 agent 类型/生命周期、Controller task 真实磁盘持久化；`TaskSnapshotStore` seam、in-memory/JSON 实现与 round-trip/损坏输入测试已完成；LocalController 已支持通过 `with_snapshot_store` 自动保存恢复（含失败 error_message 与 working session 冲突索引）；原始 order 序号持久化仍缺；父子链接持久化及重挂载/删除 child 的旧索引清理已验证，link_parent 锁重入已修复；working/parent-child 索引恢复与确定性 pending 排序已验证；app 已默认配置 JSON task snapshot 路径并通过 plugin mount 测试；跨进程并发锁语义（当前为 lock-file 冲突显式失败，无等待/租约恢复；controller 全量 22 项单测已通过；失败 replace 后 lock 清理已验证；versioned envelope、未知版本拒绝、malformed envelope 显式错误与 legacy 裸数组读取已验证；更高版本迁移仍缺）、路径 profile 配置字段和完整 app boot 验证仍缺；同实例写入已串行化；损坏、不一致和非法字段 snapshot 已显式阻止 controller 挂载、LLM 意图识别和更丰富结构化 payload 仍缺；文本控制命令成功/失败/非法 task-id 结果已写入 System session event，并保留原始 command/intent，有 focused persistence test（含 controller failure）；application 已可通过 AgentRequest.restore_checkpoint 调用 SessionManager::restore；Controller retry seam 与 `retry:<task_id>` application 路由已落地（Failed→Submitted、清除 error_message、snapshot 持久化、非法状态/未知任务显式错误）；缺失 checkpoint 显式失败，checkpoint TTL/版本兼容/过期、并发访问和复杂 agent 恢复后续执行仍缺；空/缺失 checkpoint 已显式校验；AbilityManager 已有注册/启停/skill 执行、JSON 状态持久化重载；损坏状态和缺失 skill 依赖显式失败；仍缺执行中的取消、跨 provider timeout 和完整恢复语义 |
| session | partial | append-only 日志 + 多会话 create/open/fork/list + checkpoint/fork/restore;stream 管道(emitter/manager/writer) | vcs(adapter/backend/codec/config/delta/jsonl_backend/kv_backend/manager/models/protocol)、tracer(data/decorator/handler/span/tracer/workflow_tracer)、interaction(base/interaction/interactive_input)、state(agent_state/workflow_state)、session_controller(chain_session/global_controller/data_container/scope/scope_factory/schema/utils)、checkpointer(base/checkpointer/inmemory/persistence)、agent/agent_team/workflow 内部包装与 node/store |
| common | partial | 错误模型/日志类型 | background_tasks、task_manager、clients(注册表)、constants/schema/security/utils 的完整语义 |
| foundation/llm(11 provider) | partial | openai-compatible + anthropic + SSE 流式 + json-parser | dashscope、deepseek 及其余 provider;llm/schema、llm/utils |
| foundation/prompt | partial | {{var}} 版本化渲染 + prompt-builder(18 系统提示 section + workspace/context/tools 动态 section + sanitize/report) | 结构化 schema prompt、assemble(variables)、auth |
| foundation/store | partial | kv/message 文件 + Redis + PostgreSQL 后端 | graph(milvus)、vector(chroma/milvus/gauss)、object(S3/aioboto)、db 默认实现、index(simple_memory_index)、query(chroma/milvus query + registry)、vector_fields、base_embedding/base_memory_index/base_reranker/base_vector_store |
| foundation/tool | partial | Tool + ToolRegistry + 真实工具(web/code/fs/shell/memory/retrieval/graph/mcp/subagent) | auth、form_handler、function 完整语义、service_api、utils、mcp client 变体 |
| workflow(78 类) | partial | 引擎 + Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + Http/Intent/Questioner + 检查点续跑 + LLM 节点流式消费 | condition 组件(array/number/expression)、llm/react(react_component/react_config/react_executable)、resource 组件(knowledge_retrieval/memory_retrieval/memory_write)、branch_router/branch_comp 细化、loop callback(intermediate_loop_var/loop_callback/output)、intent_detection_comp |
| graph(53 类) | partial | Pregel 超级步引擎(missing/present/equals 条件 + halt + 上限) | graph 基础(atomic_node/vertex/executable/graph_state)、store(inmemory/serde)、stream_actor(base/manager)、visualization(drawable*) |
| controller(57 类) | partial | 任务 CRUD/状态机/优先级/父子防环/执行器注册表/确定性意图；Failed 等终态恢复显式拒绝 | LLM 意图识别;legacy(config/event/reasoner/task)、modules、schema |
| operator(8 类) | partial | LLM/tool/memory/skill 四算子 + freeze + 回调 + 检查点 | legacy(llm_call) |
| runner(46 类) | partial | 回调链(优先级/retry/timeout/break/rollback + 指标);资源标签管理(TagMgr) | resources_manager(agent/tool/model/workflow/sys_operation 管理器与取消)、drunner(dmessage_queue/dsubscription/remote_client/server_adapter)、spawn |
| context_engine(72 文件) | partial | 预算组装/摘录压缩+LLM 总结/offload/reinject + 精确 tokenizer(BPE-lite) | forked 处理器(compressor/offloader/rule_compression)、token 子模块、schema、向量化 |
| memory(104 文件) | partial | JSON 文件记忆 + 图记忆(实体/关系/episode)+ memory-lite 原语(frontmatter/chunk/write) | manage(21 文件:index/mem_model/search/update)、migration(14 文件:migrator/operation)、process(extract/refine)、external(8 文件)、dreaming、codec、common、config、prompts |
| retrieval(84 文件) | partial | BM25 + 本地确定性向量(哈希 n-gram TF + 余弦)+ rerank(词法+向量融合+多样性惩罚) | embedding(7)、indexing(38 文件:indexer/processor chunker·extractor·parser·splitter)、query_rewriter、vector_store(6)、retriever(7)、reranker 细化、外部模型 embedding |
| security(20 类) | partial | 规则 guardrails + pre-execute rail | LLM 后端、builtin guardrail 上下文/模型/enums/backends |
| sys_operation(56 类) | partial | 本地 fs/shell/code(隔离 scratch + python3)/sandbox 策略 | 远程沙箱 provider(9 个:AIO/jiuwenbox/yuanrong 等)、protocal、gateway/launchers |

## 2. harness(1613 符号 / 337 文件)

| Python 子模块 | 状态 | 已落地 | 未完成(缺口) |
| --- | --- | --- | --- |
| tools(117 文件/638 符号) | partial | web_fetch/run_code/read_file/write_file/list_dir/run_shell/remember/recall/forget/ingest_knowledge/search_knowledge/graph_* 工具/mcp/delegate_task;tools-metadata(27 工具双语描述+schema) | browser_move(playwright/drivers/controllers/clients)、lsp_tool、mobile_gui(rails/skill_branch)、multimodal、worktree、cron、powershell、paid_search、tool_discovery、agent_mode_tools、ask_user、todo、goal、coding_memory、compression_recall、skills 工具、subagent 完整工具面 |
| rails(57 文件/143 符号) | partial | ShellGuard/PathGuard/ToolBudget/ApprovalRail + sandbox rail + security rail | heartbeat、llm_retry、lsp、mcp、memory(memory/coding_memory/external_memory)、security(prompt_security/tool_security/base_security)、skills(skill_create/skill_use/team_skill_create/team_skill)、task_planning、task_completion、evolution(context/skill/team_context/team_skill_evolution + trajectory + approval_runtime/commands/configuration/contracts + review)、interrupt(ask_user/confirm)、progressive_tool、context_engineer(context_assemble/context_processor)、subagent(session/subagent/verification_contract/verification)、tool_call_resilience、sys_operation、agent_mode、_multimodal |
| subagents(8 文件) | partial | code/research/plan/verify(工具白名单真实强制) | browser、explore、mobile |
| goal | partial | workspace.json + 目标状态机(ah-plugins-workspace) | goal/manager.py、evaluation.py、store.py 完整语义 |
| manifest | done(第 127 回合) | ah-plugins-manifest:catalog(重名拒绝/Effect 回滚)/factory_registry(factory_ref 可逆注册)/registration(kind 路由 TOOL/RAIL/SUBAGENT)/ModelElementFactory(双源 resolve)+ ah-contracts manifest seam | 真实 harness 元素声明(builtin/harness/meta elements)的工厂接线留待各 rails/tools 模块落地 |
| kv_cache | done(第 128 回合) | ah-plugins-kv-cache:affinity/sticky/session-id 判定 + prefetch/offload/evict 信号;ah-contracts kv_cache team 部分(状态机/manageable/control domain) | 团队 registry 运行时(记录/锁/并发)留待 runtime 接线 |
| lsp | partial(第 128 回合) | ah-plugins-lsp:状态机/诊断注册表六步算法/file_uri/5 语言 server 配置 | stdio JSON-RPC 客户端(spawn/读写/握手)留待进程接线 |
| resources | partial(第 128 回合) | ah-plugins-resources:Spec 模型/MCP 归一化/模板渲染/路径校验/ExtensionParts 解析 | manifest 发现/文件读取(FS 接线) |
| schema | partial | 部分类型 | stop_condition/task/config/interaction/agent_mode/loop_event/extension_spec/build_context/deep_agent_spec/state 字段对等 |
| security(harness) | partial | ah-plugins-security 规则 | suggestions/patterns/models/host/tiered_policy/core/factory/file_guard/shell_ast/checker/files registry/extract |
| task_loop | **partial** | agent-loop、runner、queue、subagent 已覆盖部分循环/队列/委派语义 | event_manager/loop_coordinator/loop_queues/session_spawn_executor/task_loop_controller/task_loop_event_executor/task_loop_event_handler 的完整 task-loop 生命周期与事件接线仍未对等 |
| cli | done | Claude Code 风格渲染 + ah-cli 子命令 | (done,无缺口) |

## 3. agent_teams(1104 符号 / 276 文件)

核心运行时已大量落地:swarmflow 引擎、SQLite 团队运行时、7 路 dispatch、对象池+InteractGate、
成员/执行状态机、审查投票、两阶段消息渲染、调度扫描/消息组装、入站渲染、时间渲染、名册 diff、
i18n、上下文正文、外部 CLI 子进程运行时、入站格式、交互语法、模型分配器、加入描述符、
任务状态机、团队监控、可靠性检测器(reliability-burst/tools/monitor)。

| Python 子模块 | 状态 | 未完成(缺口) |
| --- | --- | --- |
| schema(81) | partial | TeamAgentSpec/DeepAgentSpec/事件体系字段对等未全 |
| messager | partial | ZMQ ROUTER/DEALER(本地 + Redis 已做) |
| external | partial | SSH(CLI 子进程已做) |
| interaction(6) | partial | 团队交互子模块细化 |
| kv_cache(5)/memory(7)/mcp(3)/security(2)/skill(2)/spawn(5)/team_workspace(5)/prompts(8)/cli(9)/harness-manifest(10) | partial | 各子模块 0% 或未全(monitor/models 例外:monitor 已落地、models 分配器已落地) |
| observability(16) | partial | claude/codex otel 桥(agent-core 新增,未移植) |
| rails(12) | partial | 团队侧 rails 细化 |
| tools(27) | partial | 团队工具面(database/locales 描述等) |
| worktree(6) | partial(第 128 回合) | ah-plugins-worktree:命名(slug+sha256)/成员状态归属判定 | git worktree 生命周期(create/remove/贡献分类)留待 git 扩展 |
| workflow(148) | done | — |
| reliability | partial | rail/handler/factory/EventAnomalyReporter 留待后续(依赖 coordination 运行时) |
| residual.rs 资产 | partial | NativeTaskBoard/Journal/BudgetLedger 仅测试引用,未接入运行路径 |

## 4. agent_evolving(602 符号 / 208 文件)—— 已高度完成

已落地:轨迹抽取/评估/优化、experience 持久化/分享/评分、信号检测(conversation/execution/
skill_creation/team)、update_execution、protocols/constant、prompts(sections/tools)、
draft_schema/索引查询/在线类型/重建/生命周期/归档/提交/工具调用链/检查点类型。

| Python 子模块 | 状态 | 未完成(缺口) |
| --- | --- | --- |
| updater(3) | **missing** | multi_dim.py、single_dim.py、protocol.py |
| agent_rl(83 文件) | partial | reward + PPO step 数学已做;缺 VERL/PPO trainer、LoRA、online gateway(app/trajectory/upstream)、online judge/scheduler/launcher/rail/inference、offline coordinator/runtime/store、storage、optimizer、config |
| optimizer | partial | 文本梯度已做;LLM 梯度缺 |
| signal | partial | 确定性检测已做;LLM 用户意图判断缺 |
| trainer | partial | 训练循环已做;prompt/tools 组件缺 |
| evaluator | done(本地判据 + LLM judge 附加) | evaluator_pipeline adapters(agents/benchmarks)、metrics 细化 |

## 5. rsi(1695 符号 / 180 文件)—— 已高度完成

已落地:orchestrator 多轮+checkpoint、evaluator(expected+轨迹)、evaluation_result_analyzer、
member_optimizer 全链路、team_skill_generator、single_harness、auto_harness 六阶段+git/CI、
dataset_generator(确定性+LLM)、dataset_curator、data_loader 分批、rsi-config。

| Python 子模块 | 状态 | 未完成(缺口) |
| --- | --- | --- |
| auto_harness | partial | 远端 PR/GitCode 发布(本地 git/CI 已做) |
| evaluation_result_analyzer | partial | LLM 深度诊断(确定性信号+归因已做) |
| team_skill_optimizer | partial | LLM 演化管线(信号映射已做) |
| storage/resource/config/schema | partial | 现 in-memory 持久化;resource 记账、schema 字段对等未全 |

## 6. extensions(340 符号 / 102 文件)

| Python 子模块 | 状态 | 已落地 | 未完成(缺口) |
| --- | --- | --- | --- |
| checkpointer(Redis) | partial(第 128 回合) | ah-plugins-checkpointer:TTL/key 构造/四存储/钩子编排(RedisStore seam 注入) | 真实 Redis 进程集成测试 |
| message_queue(Pulsar) | partial | Redis LIST+INCR+游标 | Pulsar |
| store(GaussDB/ES) | partial | PostgreSQL/GaussDB 兼容 SQL | Elasticsearch |
| sys_operation(远程沙箱 9 provider) | partial | 4 个白名单命令 | AIO/jiuwenbox/yuanrong 等远程 provider |
| external_provider(OpenAI OAuth) | done | 设备码 + 模型目录 | — |
| context_evolver | partial | 任务记忆 JSONL/检索/摘要/注入 | Milvus 向量后端 |
| a2a / tracer_otel / mcp | done | server/client+SSE / JSONL+OTLP+semconv / stdio+http | — |
| vendor_specific | **missing** | 词法 fallback | 各厂商重排/嵌入 |

## 7. dev_tools(141 符号 / 94 文件)

| Python 子模块 | 状态 | 已落地 | 未完成(缺口) |
| --- | --- | --- | --- |
| prompt_builder | partial(第 128 回合) | ah-plugins-prompt-builder-devtools:三构建器校验/解析/模板编排(LLM seam 注入) | 真实 LLM 模型接线 |
| skill_creator/evaluator | partial | 技能注册/持久化/评估(subagent 委派) | creator 脚本(skill_omni_creation)、evaluator 管线(skill_eval_pipeline/skill_judge/skill_safety_judge/skill_tester)细化 |
| agent_builder / tune / symphony | done | — | — |

## 8. 工程收尾项(B 项,非功能域)

| 项 | 状态 | 说明 |
| --- | --- | --- |
| Python/Rust differential | missing | 当前只有 5 个 Rust regression reference;独立 Python runner 与双端比较尚未建立 |
| production static composition | missing | mock gate 未验证 catalog 解析、依赖闭合、重复 provider 和环 |
| production boot smoke | missing | 尚无统一的无 mock `boot()` + `ApplicationRuntime::invoke` 门禁 |
| 跨平台验证 | missing | 尚无 Linux/Windows/macOS CI 矩阵 |
| 覆盖率门禁(≥80%) | implemented | CI 有 `--fail-under-lines 80`;当前 HEAD 百分比须引用 CI 实测 |
| Golden fixtures | implemented | 9 个 seam,仅证明 Rust 契约稳定 |
| Rust regression references | implemented | 5 个 seam,由 Rust 生成,不证明 Python parity |
| prod mock exclusion | implemented | prod profile 不含 `ah-plugins-mock`;不等于 production boot verified |

## 9. 下一步

当前统一优先级见 [`ROADMAP.md`](ROADMAP.md):

1. P0 先建立 Python/Rust differential、production static composition、production boot smoke 和可信审计数据;
2. P1 再收敛 application/agent-loop/controller/session/workflow 主链与插件生命周期;
3. P2 推进工具、rails、context、subagents、multi-agent、retrieval/memory;
4. P3 推进 evolving、RSI、外部基础设施和 vendor-specific provider。

> 维护纪律:本文件只记录缺口,不维护独立百分比或另一套优先级。状态变更必须附实现位置、
> 测试名、production verification 和 parity 状态。
