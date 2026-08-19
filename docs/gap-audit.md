# agent-core 未完成插件化功能审计(gap-audit.md)

> 审计口径:以 agent-core(Python)为源规格,逐域核对 agent-harness 当前实现(第 120 回合,
> 91 crates / 文档记录 880 tests / clippy 0 / fmt clean;本审计实测 workspace 构建通过,
> 测试抽样 71+ 个测试二进制 0 失败)。
> 状态口径(与 capability-map.md 一致):`done`=核心语义真实落地+测试证据;
> `partial`=部分落地;`missing`=无实现。
> 本文件只列 **未完全完成(partial/missing)** 项;done 项见 capability-map.md。
> 证据基线:capability-map.md(代码证据)+ 本审计对 crate 目录 / 工具 / seam 的实测核对。

## 0. 总体结论

- 第三梯队 A(teams / evolving / rsi)与内核/契约/多数 harness 插件已真实落地;
- **最大缺口集中在 `core` 域(796 文件 / 2226 符号)**:application、multi_agent、
  single_agent 的 interrupt/skills/rail、session 的 vcs/tracer/checkpointer、memory 的
  manage/migration/process、retrieval 的 indexing/embedding/vector_store 等基本未动;
- **其次为 harness 工具与 rails 长尾**:browser/lsp/mobile/multimodal/worktree/cron/
  powershell 等工具、约 15+ 条 rails、manifest/kv_cache/lsp/task_loop 子模块 0%;
- 全仓生产路径 **0 处 `todo!()`/`unimplemented!()`**,partial 项均为"留待后续"的显式
  文档化缺口,不是残桩。

## 1. core(2226 符号 / 796 文件)—— 缺口最大

| Python 子模块 | 状态 | 已落地 | 未完成(缺口) |
| --- | --- | --- | --- |
| application(llm_agent/workflow_agent) | **missing** | — | llm_agent.py/llm_controller.py/rails(invoke_result_adapter_rail、memory_rail);workflow_agent.py/workflow_controller.py/workflow_event_handler.py/workflow_task_executor.py。无 ah-plugins-core-application crate |
| multi_agent(handoff/hierarchical/msgbus) | **missing** | — | handoff 团队(container_agent/handoff_orchestrator/handoff_signal/handoff_tool/interrupt)、hierarchical_msgbus(supervisor_agent/p2p_ability_manager)、hierarchical_tools、team_runtime(message_bus/message_router/subscription_manager/communicable_agent/envelope)、team_card。无 ah-plugins-core-multi-agent crate |
| single_agent | partial | agent-loop(ReAct)已落地;prompts/builder 部分由 prompt-builder 覆盖 | interrupt(exception/handler/response/state)、skills(skill_manager/skill_util/remote_skill_util)、ability_manager、agent_callback_manager、rail(base/model_backup)、kv_cache(kv_cache_hooks)、schema(agent_card/agent_result)、agents(react_agent/multi_task_agent/react_agent_evolve) |
| session | partial | append-only 日志 + 多会话 create/open/fork/list + checkpoint/fork/restore;stream 管道(emitter/manager/writer) | vcs(adapter/backend/codec/config/delta/jsonl_backend/kv_backend/manager/models/protocol)、tracer(data/decorator/handler/span/tracer/workflow_tracer)、interaction(base/interaction/interactive_input)、state(agent_state/workflow_state)、session_controller(chain_session/global_controller/data_container/scope/scope_factory/schema/utils)、checkpointer(base/checkpointer/inmemory/persistence)、agent/agent_team/workflow 内部包装与 node/store |
| common | partial | 错误模型/日志类型 | background_tasks、task_manager、clients(注册表)、constants/schema/security/utils 的完整语义 |
| foundation/llm(11 provider) | partial | openai-compatible + anthropic + SSE 流式 + json-parser | dashscope、deepseek 及其余 provider;llm/schema、llm/utils |
| foundation/prompt | partial | {{var}} 版本化渲染 + prompt-builder(18 系统提示 section + workspace/context/tools 动态 section + sanitize/report) | 结构化 schema prompt、assemble(variables)、auth |
| foundation/store | partial | kv/message 文件 + Redis + PostgreSQL 后端 | graph(milvus)、vector(chroma/milvus/gauss)、object(S3/aioboto)、db 默认实现、index(simple_memory_index)、query(chroma/milvus query + registry)、vector_fields、base_embedding/base_memory_index/base_reranker/base_vector_store |
| foundation/tool | partial | Tool + ToolRegistry + 真实工具(web/code/fs/shell/memory/retrieval/graph/mcp/subagent) | auth、form_handler、function 完整语义、service_api、utils、mcp client 变体 |
| workflow(78 类) | partial | 引擎 + Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + Http/Intent/Questioner + 检查点续跑 + LLM 节点流式消费 | condition 组件(array/number/expression)、llm/react(react_component/react_config/react_executable)、resource 组件(knowledge_retrieval/memory_retrieval/memory_write)、branch_router/branch_comp 细化、loop callback(intermediate_loop_var/loop_callback/output)、intent_detection_comp |
| graph(53 类) | partial | Pregel 超级步引擎(missing/present/equals 条件 + halt + 上限) | graph 基础(atomic_node/vertex/executable/graph_state)、store(inmemory/serde)、stream_actor(base/manager)、visualization(drawable*) |
| controller(57 类) | partial | 任务 CRUD/状态机/优先级/父子防环/执行器注册表/确定性意图 | LLM 意图识别;legacy(config/event/reasoner/task)、modules、schema |
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
| kv_cache | **missing** | — | kv_cache_hooks(harness 与 core/single_agent、agent_teams 三处) |
| lsp | **missing** | — | core(client/diagnostic_registry/instance/manager/types/utils/file_uri/git_ignore)+ servers(registry + go/java/python/rust/typescript) |
| resources | partial | 部分类型 | extension_loader、extension_resolver |
| schema | partial | 部分类型 | stop_condition/task/config/interaction/agent_mode/loop_event/extension_spec/build_context/deep_agent_spec/state 字段对等 |
| security(harness) | partial | ah-plugins-security 规则 | suggestions/patterns/models/host/tiered_policy/core/factory/file_guard/shell_ast/checker/files registry/extract |
| task_loop | **missing** | — | event_manager/loop_coordinator/loop_queues/session_spawn_executor/task_loop_controller/task_loop_event_executor/task_loop_event_handler |
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
| worktree(6) | partial | 团队 worktree 操作 |
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
| checkpointer(Redis) | partial | SET/GET/DEL/KEYS | TTL/集群/pipeline |
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
| prompt_builder | partial | {{var}} 渲染 | meta/feedback/badcase 构建器 |
| skill_creator/evaluator | partial | 技能注册/持久化/评估(subagent 委派) | creator 脚本(skill_omni_creation)、evaluator 管线(skill_eval_pipeline/skill_judge/skill_safety_judge/skill_tester)细化 |
| agent_builder / tune / symphony | done | — | — |

## 8. 工程收尾项(B 项,非功能域)

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 差分契约(与 agent-core 对等) | partial | Rust 基线 5 seam(references/)+ differential.rs 门禁已落地;Python 参考数据待外部生成 |
| 跨平台验证 | 待办 | Linux/Windows 未跑 |
| 覆盖率门禁(≥80%) | done | llvm-cov 实测 87.94%,CI --fail-under-lines 80 |
| 契约 fixtures(G-02) | done | fixtures/ 9 seam golden + ah-app/tests/golden.rs |
| CI mock 门禁 | done | prod profile 无 mock 插件 |

## 9. 建议的下一步优先级

1. **core/single_agent + core/application + core/multi_agent**(当前 0%):这三块是 agent-core
   "可运行的 agent" 形态(ReAct 之上的 interrupt/skills/ability + llm_agent/workflow_agent 绑定 +
   handoff/hierarchical 团队),是完成度最低且用户可见性最高的缺口;
2. **core/session 的 vcs/tracer**(session-log 之上补版本化会话与 tracer);
3. **core/memory 的 manage/migration/process**(记忆生命周期,依赖 lite/graph 已落地基础);
4. **core/retrieval 的 indexing/embedding/vector_store**(依赖外部 embedding,可先做解析管线与 retriever);
5. **harness 工具长尾**(browser/lsp/mobile/worktree/cron/powershell)与 rails 长尾;
6. **agent_evolving/agent_rl 剩余**(VERL/LoRA/gateway,依赖外部训练环境);
7. **extensions 外部后端**(ES/Pulsar/远程沙箱/Milvus,依赖容器/网络环境)。

> 维护纪律:本审计与 capability-map.md / REMAINING_PLAN.md 同源;状态变更必须附测试证据
> (文件:行),禁止只改文档不改代码。
