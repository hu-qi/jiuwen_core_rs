# 对等审计基线(parity-audit.md)

> 生成时间:第 127 回合;审计基线:`b5c3548`(92 crates / 925 tests)。
> 当前(第 141 回合):109 crates;本回合 core/operator 收尾 done(5 契约 + 2 插件测试)+ agent_teams/monitor stream_logger 收尾(7 测试)。
> 方法:7 域 97 个子模块,逐模块读 Python 源码(类/函数签名)对照 Rust crate 源码(`pub fn/struct/enum/trait` + `impl`),带 file:line 证据。
> 判定标准(严格):`done` = 全部核心功能点**含 LLM 驱动能力**(LLM judge/生成/诊断、梯度优化器、embedding/reranker、多后端/多 provider/多拓扑)在 Rust 1:1 对等;Python 是 LLM 驱动而 Rust 只有确定性规则/字符串匹配/简化循环 → `partial`;Rust 无实现 → `missing`。
> 此表衡量**行为对等度**(含 LLM 能力),不是"有代码就算完成"的功能覆盖率。

## 0. 总览

| 指标 | 值 |
| --- | --- |
| **总体对等度** | **≈ 59.5%**(按 Python 文件数加权;excluded 不参与计分) |
| done / partial / missing / excluded | **6 / 89 / 0 / 2** |
> 第 142 回合:harness/security 的 Shell AST 保守回退扫描器 + 文件路径防护(file_guard)收尾(25→48);其余不变。
> 第 141 回合:core/operator 收尾 done;harness/prompts 附件 CRUD/XML 渲染/注入收尾(55→68);harness/workspace 目录构建器/校验/schema 语言变体收尾(40→55);agent_teams/monitor 的 TeamStreamLogger 收尾(80→95)。
> 第 129 回合:context 90 / config 95 / data_loader 92(仍计 partial,未达 done 判定线 100 或 LLM 无缺) |
| 上一基线(第 75 回合) | ≈ 31% |

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

## 3. 完全未实现(missing)—— 0 个

全部 11 个 missing 模块已补到至少 partial(第 127-128 回合):manifest、resources、worktree、checkpointer、kv_cache(harness)、lsp、kv_cache(agent_teams)、prompt_builder、skill_creator;rsi/resource、rsi/storage 判 excluded(见 §3b)。

## 3b. 排除(excluded)—— 2 个(Python 侧亦为 TODO 桩,无真实功能)

| 域 | 模块 | 依据 |
| --- | --- | --- |
| rsi | resource | Python `rsi/resource/manager.py:14-20` `read_text`/`resolve_path` 均为 `NotImplementedError("TODO: ...")` 桩;Rust 不硬造功能 |
| rsi | storage | Python `rsi/storage/store.py:14-32` 五个 `allocate_*`/`write_result_ref` 均为 `NotImplementedError("TODO: ...")` 桩;Rust 不硬造功能 |

## 4. 需深度推进(partial)—— 90 个

### 4.1 core(16 子模块,53%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| workflow | 85 | STREAM/TRANSFORM/COLLECT 流式组件能力 |
| controller | 65 | LLM 意图识别(现关键字匹配) |
| graph | 60 | StreamActor 流式、可视化 |
| multi_agent | 55 | 消息总线/订阅拓扑、handoff 编排 |
| context_engine | 55 | round/dialogue 压缩、会话记忆管理器 |
| single_agent | 55 | AbilityManager、中断恢复、远程技能 |
| foundation | 50 | 8 个 provider、KV cache 亲和、向量/图/对象存储 |
| application | 45 | LLM 意图检测/任务分发 |
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
| schema | 35 | DeepAgentSpec/交互/停止条件/事件模型 |
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
| schema | 86 | **ssh_transport/task graph/blueprint 校验已收尾(第 137 回合)**:ah-contracts team_schema seam(SshTransportConfig + validate_ssh_auth 认证校验;TaskOpResult/TaskCreateResult/TaskSummary/TaskDetail/TaskListResult/TaskGraphSpec/TaskGraphResult/NewTaskSpec/GraphMutationResult 纯模型;InfraRegistry transport/storage 注册表 + transport/storage_merged_params backend/db_type 注入;validate_pool_router_exclusive/external_cli_unique/review_settings/stall_settings/swarmflow_budget/reserved_names/hitt·bridge_consistency 装配期校验,对齐 ssh_transport.py/task.py/blueprint.py 确定性部分)+ ah-plugins-team-schema(InfraRegistry 可逆注册 + 内置类型惰性播种);TeamAgentSpec.build()/DeepAgentSpec 运行时装配留待后续 |
| monitor | 95 | **stream_logger 已收尾(第 141 回合)**:TeamStreamLogger 1:1(见 §2);TeamMonitor 只读视图精简:缺 get_members/get_member/get_task 单查 + MessageInfo(broadcast/is_read)+ get_messages to/from/hide_dm 过滤,留待后续 |
| workflow | 80 | avatar session 后端/concurrency governor |
| interaction | 75 | UserInbox 持久信箱、bridge 适配 |
| memory | 75 | LLM 提取、member toolkit |
| runtime | 75 | BackgroundTaskController/InteractGate |
| observability | 70 | OTLP 导出/redaction/callback handler |
| rails | 70 | approval orchestrator/plan-mode/policy |
| skill | 65 | skill CLI 子命令 |
| harness | 60 | supervisor 控制命令队列、快照 rail |
| security | 60 | permission narrowing(strictest 合并) |
| spawn | 60 | shared_resources 单例、inprocess handle |
| cli | 55 | 团队生命周期命令/TUI |
| mcp | 55 | MCP server(仅 client) |
| messager | 55 | ZMQ/WebSocket 传输 |
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
| optimization_experience_learner | 25 | ExperienceStore/Extractor/Retriever |
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
- 47.2% 与"看起来都落地了"之间的差距,就是「LLM 驱动能力」的缺失。

## 6. 维护纪律

1. 本表以 git 基线为快照;每次审计先 `git rev-parse HEAD` 记录基线。
2. 状态变更必须附 file:line 证据 + 测试证据,禁止只改表不改代码。
3. 仓库持续演进(第 75→126 回合推进了 51 回合),本表会随基线过期;重新审计前先核对 crate 数/测试数是否与基线一致。
