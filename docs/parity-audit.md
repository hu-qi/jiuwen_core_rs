# 对等审计基线(parity-audit.md)

> 生成时间:第 127 回合;审计基线:`b5c3548`(92 crates / 925 tests)。
> 方法:7 域 97 个子模块,逐模块读 Python 源码(类/函数签名)对照 Rust crate 源码(`pub fn/struct/enum/trait` + `impl`),带 file:line 证据。
> 判定标准(严格):`done` = 全部核心功能点**含 LLM 驱动能力**(LLM judge/生成/诊断、梯度优化器、embedding/reranker、多后端/多 provider/多拓扑)在 Rust 1:1 对等;Python 是 LLM 驱动而 Rust 只有确定性规则/字符串匹配/简化循环 → `partial`;Rust 无实现 → `missing`。
> 此表衡量**行为对等度**(含 LLM 能力),不是"有代码就算完成"的功能覆盖率。

## 0. 总览

| 指标 | 值 |
| --- | --- |
| **总体对等度** | **≈ 53.1%**(按 Python 文件数加权;excluded 不参与计分) |
| done / partial / missing / excluded | **5 / 90 / 0 / 2** |
> 第 129 回合:context 90 / config 95 / data_loader 92(仍计 partial,未达 done 判定线 100 或 LLM 无缺) |
| 上一基线(第 75 回合) | ≈ 31% |

## 1. 逐域对等度

| 域 | 文件数 | 对等度 | done/partial/missing |
| --- | ---: | ---: | --- |
| core | 796 | 53% | 0/16/0 |
| agent_teams | 276 | 70% | 1/24/0 |
| agent_evolving | 208 | 64% | 1/11/0 |
| extensions | 102 | 48% | 0/10/0 |
| rsi | 175(excl. 5) | 38% | 1/12/0(2 excluded) |
| harness | 337 | 45% | 2/12/0 |
| dev_tools | 94 | 41% | 0/5/0 |

## 2. 完整完成(done)—— 5 个(全是确定性算法)

| 模块 | pct | 证据 |
| --- | ---: | --- |
| agent_evolving / dataset | 95 | `dataset.rs:40/114/125/154` Case/shuffle/split/CaseLoader 1:1 |
| agent_teams / models | 90 | `ah-plugins-model-allocator/src/lib.rs:58` 四类分配器 + IntelliRouter |
| rsi / dataset_curator | 90 | `ah-plugins-dataset-curator/src/lib.rs:423/485` 决策路径 1:1 |
| harness / manifest | 90 | `ah-plugins-manifest/src/lib.rs:27/88/136/262` + `ah-contracts/src/manifest.rs:19/48/115/189` 描述符目录/工厂注册表/kind 路由注册 1:1 |
| harness / kv_cache | 90 | `ah-contracts/src/kv_cache.rs:77/94/131/186` + `ah-plugins-kv-cache/src/lib.rs:21-118` affinity/sticky/session-id 判定 + prefetch/offload/evict 信号 1:1 |

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
| operator | 88 | PreviewableOperator 预览语义 |
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
| prompts | 55 | 附件 CRUD/XML 渲染/注入(仅哈希层) |
| workspace | 40 | 目录构建器/节点管理/schema 生成 |
| schema | 35 | DeepAgentSpec/交互/停止条件/事件模型 |
| task_loop | 35 | 事件管理器/协调器/控制器/执行器 |
| cli | 35 | chat/run 交互、auto_harness 子命令 |
| rails | 30 | 任务完成/规划/重试等 LLM 型 rail |
| subagents | 30 | 7 类具体 LLM 子代理构建器 |
| security | 25 | 权限引擎/文件守卫/分层策略/Shell AST |
| tools | 25 | edit/glob/grep/todo/cron/memory 等工具 |
| goal | 25 | LLM 评估、GoalStopConfig |
| manifest | 90 | **done(第 127 回合)**:描述符目录/工厂注册表/kind 路由注册 1:1(见 §2) |

### 4.3 agent_teams(22 子模块,65%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| agent | 90 | **bridge compose/wrap 已收尾(第 130 回合)**:ah-contracts bridge_compose seam(TeamRole/BridgeMailboxInjectMode/compose_bridge_inbound/wrap_outbound_to_remote 双语纯函数)+ ah-plugins-bridge-compose;StreamController _tag_chunk 决策已补(ah-contracts stream.rs tag_chunk) |
| external | 85 | ExternalTeamClient(MCP 收件箱) |
| reliability | 85 | DeepAgentRail/handler 接线 |
| context | 90 | **已收尾(第 129 回合)**:ah-contracts team_context seam + ah-plugins-team-context(session_id set/get/reset token 可逆,对齐 context.py) |
| monitor | 80 | stream_logger 分块摘要 |
| prompts | 80 | plan-mode/bridge brief 模板 |
| schema | 80 | blueprint/ssh_transport/task graph 规格 |
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
| team_skill_generator | 25 | LLM plan/create/repair、多文件生成 |
| auto_harness | 20 | rails、LLM agent factory、pipelines、experience store |
| evaluator | 32 | **trajectory 工具已收尾(第 131 回合)**:ah-contracts rsi_evaluator seam + ah-plugins-rsi-evaluator — bounded 轨迹(truncate_text/truncate_json_like/bounded_messages/tool_summary/safe_role_file_stem/bound_llm·tool_detail)+ usage 提取(collect_successful_tool·skill_names/canonical_tool_name/collect_pre_edit_successful_usage/is_persistent_edit_step),对齐 rsi/evaluator/trajectory_paths.py + trajectory_usage.py;LLM judge/TeamEvaluator 留待后续 |
| member_optimizer | 20 | LLM role/mechanism 归因、action_groups、修复 agent |
| dataset_generator | 15 | capability graph/case spec/维度/质量评审 |
| orchestrator | 10 | 多阶段编排、usage ledger、run report |

### 4.6 extensions(9 子模块,42%)

| 模块 | pct | 主要缺口 |
| --- | ---: | --- |
| a2a | 70 | SSE/流式、加密、AgentCard 适配/转换器 |
| tracer_otel | 65 | redaction、span manager、rail 集成、WorkflowHandler |
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
| tune | 40 | CaseLoader、DefaultEvaluator、4 类优化器、ParameterSearcher |
| agent_builder | 20 | LLM 澄清/生成/意图/设计/反思、dl_transformer |

## 5. 根因与规律

- **done 只出现在确定性算法上**(dataset/models/dataset_curator 双向都是确定性规则)。
- **凡 Python 是 LLM 驱动**(LLM 澄清/生成/诊断、梯度优化器、LLM judge、embedding/reranker、9-10 个专职 agent),Rust 一律是「确定性规则 + 字符串匹配 + 简化循环」→ partial。
- 47.2% 与"看起来都落地了"之间的差距,就是「LLM 驱动能力」的缺失。

## 6. 维护纪律

1. 本表以 git 基线为快照;每次审计先 `git rev-parse HEAD` 记录基线。
2. 状态变更必须附 file:line 证据 + 测试证据,禁止只改表不改代码。
3. 仓库持续演进(第 75→126 回合推进了 51 回合),本表会随基线过期;重新审计前先核对 crate 数/测试数是否与基线一致。
