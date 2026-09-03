# 对等审计基线(parity-audit.md)

> 审计快照基线:`agent-harness@cc561c0`,`agent-core@aeb88cd8`;当前代码 HEAD:`7404142`。
> 旧快照之后已有功能提交,包括 AgentBuilder、A2A、数据加载/策展、模型分配、交互路由、入站渲染、桥接和提示附件;原文“无功能面变化”已失效。
> 本文的百分比与 `done/partial/missing` 账目仍是旧快照结果,未由当前 HEAD 的结构化账本重新计算,不得作为当前完成度。
> 当前代码复核确认:新增插件多数只有确定性子集;阶段一已将 **10 个新增插件加入 Cargo workspace members**,阶段二已全部接入 `ah-app::plugin_catalog` 与 dev/prod Profile,并通过 targeted mount/resolve/invoke/unmount 集成测试,不能升级为严格 done。
> 方法仍为按 Python 公开行为面核对 Rust seam/plugin;严格 `done` 还要求生产路径、错误/状态/持久化、差分和适用 E2E 证据。
> 当前 Python/Rust differential 只验证 `stop_condition`、`messager_inprocess` 两个 seam,共 6 个 case,另有 1 个已知差异;首批 application/agent-loop/session/controller/workflow/tools 仍未接入。
> 当前任务顺序以 `ROADMAP.md` 为准;下一次审计必须先同步当前 HEAD、workspace package 清单和插件接线状态。

## 0. 总览

| 指标 | 值 |
| --- | --- |
| **当前总体对等度** | **未重算;旧快照人工估算约 65.5%,未由完整 Python differential 验证** |
| 当前可引用的历史账目 | **done / partial / missing / excluded = 6 / 87 / 2 / 2;仅代表旧快照** |
| 当前 differential | **2 个 seam、6 个 case 一致,1 个 known divergence;首批六 Seam 未接入** |
| 当前 production boot | **dev Profile 无云凭据 boot + demo E2E 已通过;prod Profile 缺少 OpenAI key 时显式失败,完整无 mock prod boot 未完成** |
> 阶段一验证:10 个新增插件已加入 workspace,`cargo check --workspace --offline` 通过,新增插件单元测试 117 个 case 全部通过;阶段二已接入 catalog/dev/prod Profile,`stage2_plugins` 与 `static_composition`/`mock_gate` 共 8 个测试通过;阶段二第二批已通过 dev Profile 无云凭据 boot 与 application invoke 回归,并确认 prod 无 key 的显式门槛;这些结果不替代 Python differential 或完整 production boot 证据。

旧快照中的逐域百分比和回合记录保留作历史证据,不得作为当前 HEAD 完成度。

## 1. 逐域对等度

| 域 | 文件数 | 对等度 | done/partial/missing |
| --- | ---: | ---: | --- |
| core | 796 | 54% | 1/15/0 |
| agent_teams | 276 | 71% | 1/24/0 |
| agent_evolving | 208 | 64% | 1/11/0 |
| extensions | 102 | 48% | 0/10/0 |
| rsi | 175(excl. 5) | 38% | 1/12/0(2 excluded) |
| harness | 337 | 45% | 2/12/0 |
| dev_tools | 94 | 41% | 0/5/0 |

## 2. 完整完成(done)—— 6 个(全是确定性算法)

| 模块 | pct | 证据 |
| --- | ---: | --- |
| agent_evolving / dataset | 95 | `dataset.rs:40/114/125/154` Case/shuffle/split/CaseLoader 1:1 |
| agent_teams / models | 90 | `ah-plugins-model-allocator/src/lib.rs:58` 四类分配器 + IntelliRouter |
| rsi / dataset_curator | 90 | `ah-plugins-dataset-curator/src/lib.rs:423/485` 决策路径 1:1 |
| harness / manifest | 90 | `ah-plugins-manifest/src/lib.rs:27/88/136/262` + `ah-contracts/src/manifest.rs:19/48/115/189` 描述符目录/工厂注册表/kind 路由注册 1:1 |
| harness / kv_cache | 90 | `ah-contracts/src/kv_cache.rs:77/94/131/186` + `ah-plugins-kv-cache/src/lib.rs:21-118` affinity/sticky/session-id 判定 + prefetch/offload/evict 信号 1:1 |
| core / operator | 100 | **done(第 141 回合)**:`ah-contracts/src/operator.rs:71/96/128` Operator.apply_update 兼容行为(replace/state→set_parameter+前后状态比较,其余显式错误)+ PreviewableOperator(preview_update 抽象 + apply_update 路由预览)+ TunableKind::SkillExperience;`ah-plugins-operator/src/lib.rs:434-548` SkillExperienceOperator(operator_id=skill_experience_{skill},tunables experiences/kind skill_experience/path content/preview_update 目标+mode/effect 校验→records+lifecycle_stage=local_apply_completed+metadata.skill_name/set_parameter 通知消费方/get_state={}/load_state 无副作用),1:1 对齐 operator/base.py:114-181 + skill_call/base.py:22-117;ApplyResult 补 records/lifecycle_stage/pending_change_id(evolving.rs:230) |

## 3. 完全未实现(missing)—— 2 个

当前仍有两个能力子模块没有可调用的 Rust 实现:

| 域 | 模块 | 缺口 |
| --- | --- | --- |
| rsi | updater | `multi_dim.py`、`single_dim.py`、`protocol.py` |
| extensions | vendor_specific | 各厂商专用重排/嵌入实现 |

原先标记为 missing 的其他模块均已至少有 seam、插件、确定性逻辑或部分真实路径,
因此已改为 `partial`;这不代表 Python 行为已完成对等。manifest、resources、worktree、
checkpointer、kv_cache(harness)、lsp、kv_cache(agent_teams)、prompt_builder、
skill_creator 均属于此类。rsi/resource、rsi/storage 判 excluded(见 §3b)。

## 3b. 排除(excluded)—— 2 个(Python 侧亦为 TODO 桩,无真实功能)

| 域 | 模块 | 依据 |
| --- | --- | --- |
| rsi | resource | Python `rsi/resource/manager.py:14-20` `read_text`/`resolve_path` 均为 `NotImplementedError("TODO: ...")` 桩;Rust 不硬造功能 |
| rsi | storage | Python `rsi/storage/store.py:14-32` 五个 `allocate_*`/`write_result_ref` 均为 `NotImplementedError("TODO: ...")` 桩;Rust 不硬造功能 |

## 4. 需深度推进(partial)

> 本节保留历史逐域估算。标题不再复制与总览不一致的手工数量;统一计数待 P0-05 自动生成。

### 4.1 core(16 子模块,53%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| workflow | 86 | STREAM/TRANSFORM/COLLECT 流式组件能力(ComponentAbility 已收尾,第 143 回合:invoke/stream/collect/transform 四能力名+描述,对齐 base.py);流式执行管线留待后续 |
| controller | 65 | LLM 意图识别(现关键字匹配) |
| graph | 60 | StreamActor 流式、可视化 |
| multi_agent | 55 | 消息总线/订阅拓扑、handoff 编排 |
| context_engine | 55 | round/dialogue 压缩、会话记忆管理器 |
| single_agent | 58 | agent-control、callback、AbilityManager、model-backup policy 与顺序 fallback、请求级 timeout seam、session 日志恢复和 application 路由均已有部分实现与聚焦测试。主要缺口是宿主真实多 provider catalog 组装、执行中模型/工具取消、跨 provider timeout、完整 rail/kv-cache hooks、结构化运行错误和 Python differential。 |
| foundation | 50 | 8 个 provider、KV cache 亲和、向量/图/对象存储 |
| application | 58 | `ah-plugins-application` 通过 `SessionManager` 按 session_id 创建/打开持久会话，并路由 LLM agent/workflow agent；本回合新增 `AgentLoopRuntime` contracts seam，application 只解析 `dyn AgentLoopRuntime`；`AgentLoopRuntime::card()` 已通过 seam 暴露 AgentCard 能力与恢复能力，contracts 提供兼容默认值，移除对 agent-loop 具体类型的生产依赖；timeout 通过 `AgentLoopRuntime::run_in_session_with_timeout` 从请求传入，并仍为轮次边界检查，写入 AgentTimedOut event；取消/中断/超时已映射到 AgentResult 状态；agent-loop 失败会追加带 command/state/error 的 System event；Controller seam 已可选接入 application；`TaskSnapshotStore` seam 已有 in-memory/JSON 实现，LocalController 可选消费 Context snapshot provider 或显式 JSON 路径，ah-app 默认使用 workspace/controller/tasks.json，显式 path plugin mount 测试通过；自动保存/恢复（含失败 error_message、working/parent-child 索引、父子链接持久化（含重挂载/删除 child 的旧索引清理，并避免 link_parent 锁重入）与确定性 pending 排序）、round-trip、损坏输入、不一致/非法字段快照、损坏挂载失败和同实例并发写入与跨实例 lock 冲突显式失败测试已通过；controller 全量 21 项单测通过；versioned envelope、未知版本拒绝、malformed envelope 显式错误与 legacy 裸数组兼容已验证；controller 全量 22 项单测通过，失败 replace 后 lock 清理已验证；controller 全量 19 项单测通过；文本 task-id 控制命令已执行，支持 `verb:task-id` 与自然语言 `task-...` 标识，映射错误并写入 System session event（含 retry:<task_id> 失败任务重置、错误清理与持久化；resume:<task_id> 更新 Submitted 后重新调用 Controller::run_task；非法状态/未知任务显式错误；controller 24 项、application 12 项 focused tests 通过）（保留原始 command 与 intent，非法 task-id 也持久化失败事件），focused persistence tests 通过（含 controller failure），LLM 意图检测和更丰富的结构化 command payload 仍缺（当前已持久化 raw command/intent）；Failed 终态恢复拒绝已由状态机测试覆盖；AgentRequest 已支持命名 checkpoint restore；application 缺失/空 checkpoint failure 与 workflow restore 集成测试通过，JSONL restore persistence test 已有，并覆盖非法 checkpoint 名称/路径穿越 |
| runner | 45 | 装饰器框架、Pulsar、drunner、资源管理器 |
| sys_operation | 45 | 沙箱网关/容器隔离、code 操作、进程注册表 |
| common | 40 | HTTP/LLM 客户端池、后台任务、日志异常体系 |
| memory | 40 | 外部 provider 多后端、LLM 图抽取、dreaming/迁移 |
| security | 40 | LLM/API/本地模型 guardrail 后端 |
| session | 40 | 状态提交/回滚、checkpointer、tracer、VCS、流 |
| retrieval | 35 | 向量库/embedding 多后端、LLM reranker、agentic/graph retriever |

### 4.2 harness(10 子模块,24%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| prompts | 68 | **附件 CRUD/XML 渲染/注入已收尾(第 141 回合)**:ah-contracts prompt_attachment 补渲染纯函数(xml_text/xml_attr/kind_value/stable_sort_key/is_expired/render(DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS=12000/DEFAULT_MAX_RENDERED_CHARS=48000,`<system-reminder>` 块/单附件超限截断标记/总量截断)/inject_messages)+ PromptAttachmentStore seam(add_section/clear_section/get_by_id/update_by_id/remove_by_id/list_by_filter/remove_by_filter(无过滤+allow_all=false 显式错误)/clear_session/clear_all/collect_for_session(过期剔除)),对齐 prompt_attachment_manager.py:74-661;ah-plugins-prompt-attachment InMemoryPromptAttachmentStore(真实实现:section_id 净化/id=session.{safe}.{safe}/metadata 合并 {section,source}/normalize_for_write(UTC 时间戳+内容 sha256+metadata.section)/update 不可变字段回写/稳定排序 (priority,source,section));XML 转义/CRUD/过滤/过期/排序/注入 6 契约 + 6 插件测试;PromptAttachmentContextWriter(上下文会话绑定)与 make_window_mutator 留待上下文接线 |
| workspace | 55 | **目录构建器/校验/schema 语言变体已收尾(第 141 回合)**:ah-contracts workspace 补 workspace_schema(cn/en 双语言,对齐 DEFAULT_WORKSPACE_SCHEMA/_EN,含 context 节点)+ validate_directory_node(非 dict/name 空·含分隔符/path·description 类型/is_file·default_content 类型/children 递归,对齐 _validate_directory_node)+ node_full_path(顶层路径拼接,对齐 get_node_path)+ set_directory(同名替换,对齐 set_directory)+ is_safe_relative_path(绝对/盘符/UNC/.. 越级拒绝,对齐 directory_builder.py `_is_safe_path`);ah-plugins-workspace DirectoryBuilder(真实递归建目录+`.workspace` 标记+文件默认内容,不安全路径显式 `Unsafe path detected` 不落盘);4 契约 + 3 插件测试;链接管理(.team/.worktree 软链)与语言感知默认内容留待后续 |
| schema | 45 | **停止条件已收尾(第 143 回合)**:ah-contracts harness_schema(StopEvaluationContext(iteration/token_usage/elapsed_seconds/last_result/extra)+ MaxRoundsEvaluator(iteration>=max)/TokenBudgetEvaluator(token_usage>=max)/TimeoutEvaluator(elapsed>=timeout)/CompletionPromiseEvaluator(连续确认计数,notify_fulfilled·notify_absent 打断·reset·get_state/load_state(fulfilled = count>=required 或已有 fulfilled),对齐 stop_condition.py:20-223));5 契约测试;DeepAgentSpec/交互/事件模型留待后续 |
| task_loop | 35 | 事件管理器/协调器/控制器/执行器 |
| cli | 35 | chat/run 交互、auto_harness 子命令 |
| rails | 30 | 任务完成/规划/重试等 LLM 型 rail |
| subagents | 30 | 7 类具体 LLM 子代理构建器 |
| security | 48 | **Shell AST + 文件路径防护已收尾(第 142 回合)**:`ah-plugins-security/src/shell_ast.rs` 保守回退扫描器(parse_shell_for_permission:空→simple/风险结构(管道·复合·替换·展开·heredoc·重定向)→parse_unavailable/shlex 风格 argv(shlex_split_posix 单引号·双引号转义·反斜杠·未闭合→None)/ShellStructureFlags.has_risky_structure/运算符标记收集,对齐 shell_ast.py:34-186)+ `file_guard.rs`(PermissionLevel/PermissionResult/FileGuardMode/Match/Action/AxisDefaults/PathRule/EffectiveFileGuardConfig 纯类型 + parse_level/strictest/axis_from_star/apply_implications(Write|Exec⇒Read,显式 deny 优先)/compile_path_entry(prefix 无 / 跳过)/match_glob(手写 **/ */? 分段匹配)/looks_like_path + normalize_path_guard_config(enabled 判定 + native/legacy 分支)+ FileGuardChecker(legacy workspace 隐式放行 + external_directory 前缀 / native defaults+prefix+glob+workspace 轴 + trusted_dirs + 迁移源 / resolve_one(最长前缀·glob 命中·deny>ask>allow·未命中 defaults)/evaluate(全 ALLOW→None,拒绝/待批 reason+matched_rule)/collect_ask_accesses/extract_paths_legacy(写类工具→write 轴,shell 命令路径抽取)),对齐 file_guard.py:81-747 + models.py + tiered_policy.py 确定性部分);8 契约 + 7 插件测试;tiered_policy 工具级规则/权限引擎组合/approval_overrides 留待后续 |
| tools | 25 | edit/glob/grep/todo/cron/memory 等工具 |
| goal | 25 | LLM 评估、GoalStopConfig |
| manifest | 90 | **done(第 127 回合)**:描述符目录/工厂注册表/kind 路由注册 1:1(见 §2) |

### 4.3 agent_teams(22 子模块,65%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| agent | 90 | **bridge compose/wrap 已收尾(第 130 回合)**:ah-contracts bridge_compose seam(TeamRole/BridgeMailboxInjectMode/compose_bridge_inbound/wrap_outbound_to_remote 双语纯函数)+ ah-plugins-bridge-compose;StreamController _tag_chunk 决策已补(ah-contracts stream.rs tag_chunk) |
| external | 90 | **ExternalTeamClient 已收尾(第 135 回合)**:ah-contracts external_client seam(InboxView/InboxMessage/compose_inbox_text/ExternalInboxSource/ExternalTeamClientFactory/ExternalTeamClient — 描述符投影/session 绑定/幂等 connect·close/未连接显式报错/fetch_inbox/read_inbox,对齐 client.py)+ ah-plugins-external client.rs;read_inbox 组合 external-format + team-message 模板展开 + team-i18n 看板文案;watch(messager 订阅)留待 messager seam |
| reliability | 90 | **rail/handler/factory 已收尾(第 136 回合)**:ah-contracts reliability_rail seam(error_text/args_as_dict/measure_response 纯辅助 + 6 个 hook 信号构造 + format_anomaly_event/format_anomaly 一行摘要 + route_decision 策略路由决策 + member_detector_specs enabled 装配决策 + ReliabilityRail/ReliabilityHandler/ReliabilityFactory seam,对齐 rail.py/handler.py/factory.py 确定性部分)+ ah-plugins-reliability-monitor rail.rs(MemberReliabilityRail hook→monitor→LocalAutoRemediator 纠偏 + bind_local_sink 本地上报;LeaderReliabilityHandler 路由+格式;ReliabilityAssembly 规格+策略视图);事件订阅/投递(协调运行时)留待后续 |
| context | 90 | **已收尾(第 129 回合)**:ah-contracts team_context seam + ah-plugins-team-context(session_id set/get/reset token 可逆,对齐 context.py) |
| prompts | 90 | **loader.py 已收尾(第 135 回合)**:ah-contracts team_prompts 增 TeamPromptLoader seam + ah-plugins-team-prompts EmbeddedTeamPromptLoader(嵌入 scheduler_* 双语模板,缺失显式 Err,对齐 loader.py load_template);plan-mode/bridge brief 已收尾(第 134 回合):team_plan_mode 双语模板渲染 + bridge brief;messages/sections 装配留待后续 |
| schema | 89 | **事件主题/消息 topic 已收尾(第 143 回合)**:ah-contracts team_schema 补 TeamTopic(team/task/message + build `session:{sid}:team:{team}:{topic}`)+ swarmflow_human_reply_topic(run_id 作用域/legacy)+ format/parse_swarmflow_human_reply_target(冒号数区分 run-scoped vs legacy,对齐 events.py:24-90);ssh_transport/task graph/blueprint 校验已收尾(第 137 回合):ah-contracts team_schema seam(SshTransportConfig + validate_ssh_auth 认证校验;TaskOpResult/TaskCreateResult/TaskSummary/TaskDetail/TaskListResult/TaskGraphSpec/TaskGraphResult/NewTaskSpec/GraphMutationResult 纯模型;InfraRegistry transport/storage 注册表 + transport/storage_merged_params backend/db_type 注入;validate_pool_router_exclusive/external_cli_unique/review_settings/stall_settings/swarmflow_budget/reserved_names/hitt·bridge_consistency 装配期校验,对齐 ssh_transport.py/task.py/blueprint.py 确定性部分)+ ah-plugins-team-schema(InfraRegistry 可逆注册 + 内置类型惰性播种);TeamAgentSpec.build()/DeepAgentSpec 运行时装配留待后续 |
| monitor | 95 | **stream_logger 已收尾(第 141 回合)**:TeamStreamLogger 1:1(见 §2);TeamMonitor 只读视图精简:缺 get_members/get_member/get_task 单查 + MessageInfo(broadcast/is_read)+ get_messages to/from/hide_dm 过滤,留待后续 |
| workflow | 80 | avatar session 后端/concurrency governor |
| interaction | 75 | UserInbox 持久信箱、bridge 适配 |
| memory | 75 | LLM 提取、member toolkit |
| runtime | 80 | **BackgroundTaskController 已收尾(第 144 回合)**:ah-contracts team_pool 补 SwarmflowRunHandle(abort/abort_sessions/cancel/relaunch 闭包注入,对齐 SwarmflowRunHandle)+ BackgroundTaskController(register/deregister 幂等/pause 三步(abort→abort_sessions→cancel)+ 登记 relaunch/resume 重放/is_paused,空集 no-op 返回 false,对齐 background_task_controller.py:26-119);3 契约测试;InteractGate(既有);其余 runtime 留待后续 |
| observability | 70 | OTLP 导出/redaction/callback handler |
| rails | 70 | approval orchestrator/plan-mode/policy |
| skill | 65 | skill CLI 子命令 |
| harness | 60 | supervisor 控制命令队列、快照 rail |
| security | 72 | **permission narrowing 已收尾(第 145 回合)**:ah-contracts security 补 narrow_permissions(逐工具 strictest(base,override):tools 显式级别优先,否则 defaults[tool] → defaults["*"] → ASK 兜底;只收紧不放宽;其余字段保留,对齐 narrowing.py:19-63)+ format_base_permissions_for_desc(cn/en 双语规则清单:显式工具 + defaults["*"] 兜底行 + 收窄规则说明,对齐 narrowing.py:66-131);3 契约测试;其余留待后续 |
| spawn | 60 | shared_resources 单例、inprocess handle |
| cli | 55 | 团队生命周期命令/TUI |
| mcp | 55 | MCP server(仅 client) |
| messager | 65 | **base 配置模型 + inprocess 传输已收尾(第 146 回合)**:ah-contracts messager seam(MessagerPeerConfig/MessagerTransportConfig(backend=inprocess 默认/team_name=default/node_id/地址字段/request_timeout=10.0+broadcast_topic `team:{team}:broadcast`,对齐 base.py:19-57)+ SubscriptionHandle + create_messager(inprocess→InProcessMessager,其他后端显式 `Unsupported messager backend: {backend}`,对齐 base.py:65-78)+ InProcessBus(topic→agent_id→handler pub-sub/unsubscribe 空桶清理/p2p register·send(无 handler→false)/clear,对齐 inprocess.py:21-75)+ InProcessMessager(publish 空 sender_id 盖章为 agent_id/subscribe·unsubscribe·send/register·unregister_direct_message_handler,对齐 inprocess.py:101-152));5 契约测试;pyzmq 跨进程传输留待外部依赖 |
| tools | 55 | build_team/spawn/task/approve 团队工具 |
| team_workspace | 50 | mount/文件锁/冲突策略(语义不同) |

### 4.4 agent_evolving(11 子模块,64%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| signal | 80 | LLM detect_user_intent(现规则 fallback) |
| evaluator | 70 | MetricEvaluator、LLM 主导评分 |
| experience | 70 | LLM 评分与库整理、approve/reject/stage |
| sharing | 70 | LLM 关键词提取、远程 hub_client |
| trainer | 70 | forward/predict、checkpoint 恢复、updater 绑定 |
| checkpointing | 65 | FileCheckpointStore 持久化、merge_records、skill 打包 |
| updater | 60 | SingleDim/MultiDimUpdater 绑定、状态持久化 |
| optimizer | 55 | LLM 梯度生成与 prompt 改写 |
| agent_rl | 45 | VERL/LoRA/gateway、reward 注册表 |
| trajectory | 45 | TrajectoryBuilder/Store/Registry、OTLP trace 处理 |
| tools | 40 | 12 个进化评审工具执行实现 |

### 4.5 rsi(12 子模块,33%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| config | 95 | **已收尾(第 129 回合)**:ah-plugins-rsi-config loader.rs(默认模板引导/缺失显式 FileNotFoundError/非 mapping 显式 ValueError/team_spec 路径绝对化,对齐 loader.py) |
| data_loader | 92 | **已收尾(第 129 回合)**:ah-plugins-data-loader BatchPlanStore(dataset_profile.yaml/batch_plan.yaml 真实落盘,对齐 plan_store.py) |
| single_harness | 40 | iterative 能力门禁/物化/verifier delta |
| evaluation_result_analyzer | 42 | **信号提取已收尾(第 132 回合)**:ah-contracts analyzer 补充 fingerprint_error(ts/uuid/path/hex/:N 五模式替换+空白归一,对齐 signal_extractor.py)+ extract_generic_signals(exec/judge 失败/错误聚类/expected mismatch/missing reference,对齐 GenericSignalExtractor)+ EvaluationSummaryInput/CaseAnalysisInput/DeterministicSignals 类型;ah-plugins-rsi analyzer 集成 error_clusters;LLM 两阶段诊断/Pytest/Reward/Atomic/LlmJudge 提取器留待后续 |
| team_skill_optimizer | 30 | LLM experience_optimizer、evolve_and_rebuild |
| optimization_experience_learner | 50 | **索引检索/净化/状态机已收尾(第 143 回合)**:ah-contracts rsi_learner seam(VALID_STATUSES/DEFAULT_RETRIEVAL_STATUSES/SENSITIVE_KEYS 常量 + confidence_score/truncate(`...\[truncated\]`)/bounded_list/string_list/status_value/safe_name/sanitize_value(sk-·Bearer→[redacted],敏感键剔除)/first_mapping/first_text/merge_dicts/allowed_statuses/entry_matches_query,对齐 learner.py:26-36+687-801)+ ah-plugins-rsi experience_learner.rs(ExperienceRetriever:index.yaml 真实 YAML 加载/状态·类型·阶段·角色·候选模块·失败签名·机制类型过滤/(confidence,created_at) 降序/limit 截断/summary 预算截断 bounded_match(读 stage YAML 组装扁平视图)/read_structured(json/yaml),对齐 learner.py:515-628+655-694);7 契约 + 4 插件测试;LLM 驱动的 learn/ExperienceExtractor 提取与 ExperienceStore 写路径留待后续 |
| team_skill_generator | 40 | **确定性归一化已收尾(第 133 回合)**:ah-contracts team_skill_generator seam(plan_slugify/single_line/string_list/normalize_roles(≥2 角色/id 去重/kind 校验/缺省回退)/normalize_workflow_steps(executor 校验+默认两步)/normalize_team_skill_plan(team_name/description/acceptance 回退)/write_skill_md 骨架,对齐 generator.py 确定性部分)+ ah-plugins-team-skill-generator;LLM plan/create/repair、多文件生成、验证留待后续 |
| auto_harness | 20 | rails、LLM agent factory、pipelines、experience store |
| evaluator | 32 | **trajectory 工具已收尾(第 131 回合)**:ah-contracts rsi_evaluator seam + ah-plugins-rsi-evaluator — bounded 轨迹(truncate_text/truncate_json_like/bounded_messages/tool_summary/safe_role_file_stem/bound_llm·tool_detail)+ usage 提取(collect_successful_tool·skill_names/canonical_tool_name/collect_pre_edit_successful_usage/is_persistent_edit_step),对齐 rsi/evaluator/trajectory_paths.py + trajectory_usage.py;LLM judge/TeamEvaluator 留待后续 |
| member_optimizer | 20 | LLM role/mechanism 归因、action_groups、修复 agent |
| dataset_generator | 15 | capability graph/case spec/维度/质量评审 |
| orchestrator | 10 | 多阶段编排、usage ledger、run report |

### 4.6 extensions(9 子模块,42%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| a2a | 78 | **转换器/AgentCard 适配/客户端聚合已收尾(第 138 回合)**:ah-contracts a2a seam(to_a2a_request(openjiuwen dict→SendMessageRequest,conversation_id/sessionId→context_id,metadata 过滤 null·排除 query)、message_to_payload、a2a_status_to_ojw(A2A TaskState 九态映射)、a2a_part↔A2A part 转换(data dict→struct_value/标量→string_value)、a2a_artifact_to_artifact、a2a_task/message_to_result、merge_agent_results(artifacts 拼接/metadata 合并/状态优先级)+ with_session_id、resolve_session_id、normalize_jsonrpc_route_path/interface_url(尾斜杠)、resolve_transport_protocols(gRPC 显式拒绝)、to_a2a_agent_card(描述拼接 [input_params]/[output_params]+ interfaces 构建),对齐 a2a_transformer.py/a2a_agentcard_adapter.py/a2a_client.py/a2a_server.py 确定性部分)+ ah-plugins-a2a(A2AAdapter 门面);SSE 流式/加密传输留待 transport 扩展 |
| tracer_otel | 75 | **redaction/config/semconv/span manager/setup 决策/属性映射已收尾(第 139 回合)**:ah-contracts tracer_otel seam(OtelTracerConfig 全量配置 + validate_sample_rate ∈ [0,1];truncate/hash_value(sha256:+16 hex)/should_redact(redact_prompts·completions 细粒度覆盖)/redact;semconv 属性键常量(gen_ai.*/openjiuwen.workflow.*/openjiuwen.agent.*);OtelAgentSpanManager/OtelWorkflowSpanManager(push/pop/get + on_invoke_data·stream_inputs·stream_outputs 缓冲);resolve_exporter(console/otlp + grpc/http + http endpoint 补 /v1/traces + BatchSpanProcessor 参数,未知类型显式 Err);workflow_attrs/workflow_call_start_attrs(根/组件双分支 + loop 属性)/format_elapsed(ms/s)/is_workflow_root/span_kind_for_component(LLM→Client)/workflow_span_name/resolve_parent_context(四分支)/is_llm·tool_component(子串匹配),对齐 config.py/redaction.py/semconv.py/span_manager.py/setup.py/handler.py 确定性部分)+ ah-plugins-tracer-otel(OtelTracer 门面);真实 OTel SDK 导出器/rail 集成(核心 session/tracer 基础设施)留待后续 |
| external_provider | 55 | 多 provider 注册表、多模型目录 |
| message_queue | 55 | Pulsar 后端 |
| store | 50 | ES 向量存储、GaussDB 方言 |
| sys_operation | 40 | JiuwenBox/Yuanrong 远程 provider |
| context_evolver | 30 | Milvus/多算法/轨迹生成/演化 Agent |
| vendor_specific | 30 | Dashscope/Aliyun 云端适配 |
| harness | 10 | Python 侧为空命名空间 |

### 4.7 dev_tools(3 子模块,24%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| skill_evaluator | 45 | 多 skill 流水线(tester/judge/safety)、报告落盘 |
| tune | 52 | **确定性数据模型/工具已收尾(第 140 回合)**:ah-contracts tune_kit seam(TuneConstant 常量 + Case/EvaluatedCase 模型;CaseLoader(shuffle 确定性 LCG/split 比例切分/assign_case_id,对齐 dataset/case_loader.py);TuneUtils(validate_digital_parameter 范围校验/parse_json_from_llm_response```json```块提取/parse_list_from_llm_response```list```块提取/convert_cases_to_examples few-shot 格式化/convert_dict_to_string,对齐 utils.py);TextualParameter(gradient 存取)/TraceNode/OptimizeHistory(case_id 轨迹 + get_llm_call_history,对齐 optimizer/base.py);Progress(run_epoch/run_batch + best_batch_score 重置,对齐 trainer/base.py);extract_optimized_prompt_from_response(标签提取 + prompt_base 去除)/find_placeholders/find_missing_placeholders/create_bad_case_text(对齐 instruction_optimizer.py 确定性部分)/evaluate_result_to_score(true→1.0,对齐 evaluator.py))+ ah-plugins-tune kit.rs(TuneKit 门面,注册 tune-kit 键);LLM 驱动梯度生成/优化器 backward/DefaultEvaluator 模型调用留待后续 |
| agent_builder | 20 | LLM 澄清/生成/意图/设计/反思、dl_transformer |

## 5. 根因与规律

- **done 只出现在确定性算法上**(dataset/models/dataset_curator 双向都是确定性规则)。
- **凡 Python 是 LLM 驱动**(LLM 澄清/生成/诊断、梯度优化器、LLM judge、embedding/reranker、9-10 个专职 agent),Rust 一律是「确定性规则 + 字符串匹配 + 简化循环」→ partial。
- 历史结构覆盖估算与严格对等之间的差距,主要来自 LLM 驱动能力、外部后端、生产组合验证和 Python differential 缺失。

## 6. 维护纪律

1. 本表以 git 基线为快照;每次审计先 `git rev-parse HEAD` 记录基线。
2. 状态变更必须附 file:line 证据 + 测试证据,禁止只改表不改代码。
3. 仓库持续演进(第 75→126 回合推进了 51 回合),本表会随基线过期;重新审计前先核对 crate 数/测试数是否与基线一致。
