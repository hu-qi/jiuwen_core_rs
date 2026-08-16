# 完成度看板(progress-dashboard.md)

> 生成时间:第 62 回合;代码证据:61 crates / 38,798 行 Rust / 331 tests(307 单元 + 24 集成)。
> 三态口径:done=核心语义真实落地+有测试证据(100%) / partial=部分落地(计 50%) / missing=无实现(0%)。
> 域完成度 =(done + 0.5×partial) / (done+partial+missing);总体按 Python 文件数加权。
> 注意:此看板衡量「功能落地率」,不是「行为对等率」(差分契约 golden 目前仅覆盖 9 seam fixtures + 5 seam 基线)。

## 0. 总览

| 指标 | 值 |
| --- | --- |
| 总体完成度 | **≈ 54%(加权中值,区间 50–60%)** |
| crate 数 | 61 |
| Rust 行数 | 36,984 |
| 测试数 | 331(307 单元 + 24 集成) |
| seam 契约 | 50+ ServiceKey,全部就位 |
| 生产路径 stub | 0 处 `todo!/unimplemented!` |

## 1. 逐域看板

### 1.1 core(796 文件)→ 52.6%(done 3 / partial 14 / missing 2)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| foundation/tool | done | 工具调用链真实执行 |
| graph/Pregel | done | 超级步引擎 + 条件路由 + 中断 |
| operator | done | LLM/tool/memory/skill 四算子 + freeze + 检查点 |
| common | partial | 错误模型/日志类型有;后台任务缺 |
| foundation/llm | partial | openai+anthropic+流式+json-parser 已做;dashscope/deepseek 缺 |
| foundation/prompt | partial | {{var}} 渲染已做;结构化 schema 缺 |
| foundation/store | partial | 文件/Redis/PG 已做;ES/Milvus/Chroma 缺 |
| workflow | partial | 引擎+节点+检查点+流式已做;react/condition/resource 组件缺 |
| controller | partial | 确定性已做;LLM 意图缺 |
| runner | partial | 回调链已做;资源管理/取消缺 |
| session | partial | 日志+fork/checkpoint/restore 已做;VCS/tracer 缺 |
| context_engine | partial | 预算+压缩+offload+精确 tokenizer 已做;向量化缺 |
| memory | partial | JSON+graph 已做;lite/coding/外部 provider 缺 |
| retrieval | partial | BM25+向量+rerank 已做;embedding/多 retriever/解析管线/外部向量库缺 |
| security | partial | 规则已做;LLM 后端缺 |
| sys_operation | partial | 本地已做;远程沙箱缺 |
| single_agent | partial | ReAct 已做;interrupt/skills/ability/rail/kv_cache 缺 |
| application | missing | llm_agent / workflow_agent 0% |
| multi_agent | missing | handoff / hierarchical / msgbus 0% |

### 1.2 harness(337 文件)→ 42.3%(done 2 / partial 7 / missing 4)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| cli | done | Claude Code 风格渲染器 + REPL + 会话存储 |
| workspace | done | workspace.json + 目标状态机 |
| tools | partial | web/code/fs/shell/todo/memory/retrieval/subagent/mcp/skill/graph 已做;browser/lsp/mobile/multimodal/worktree/cron/powershell/paid_search 缺 |
| rails | partial | ShellGuard/PathGuard/ToolBudget/ApprovalRail 已做;heartbeat/llm_retry/lsp/mcp/memory/security/skills/task_*/evolution/interrupt 缺 |
| subagents | partial | code/research/plan/verify 已做;browser/explore/mobile 缺 |
| goal | partial | 目标状态机有;manager/evaluation/store 缺 |
| resources | partial | 部分类型;manager 未完整 |
| schema | partial | 类型有;字段对等未全 |
| security | partial | 有 security 插件;LLM 后端缺 |
| manifest | missing | 0% |
| prompts | missing | 0%(仅 prompt 注册表) |
| kv_cache | missing | 0% |
| lsp | missing | 0% |

### 1.3 agent_teams(276 文件)→ 50.0%(done 1 / partial 6 / missing 1)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| workflow | done | swarmflow 引擎(phase/agent barrier/预算/journal) |
| runtime | partial | 内存+SQLite 已做;池/7 路 dispatch 缺 |
| external | partial | CLI 子进程已做;SSH 缺 |
| messager | partial | 本地+Redis 已做;ZMQ/hybrid 缺 |
| coordination/scheduling | partial | swarmflow 已做;生命周期/调度细化缺 |
| schema | partial | TeamAgentSpec/事件字段对等未全 |
| kv_cache/memory/monitor/models/rails/skill/prompts/cli/harness(20+ 子模块) | partial | monitor 已做;其余 ~20 个子模块 0% |
| residual.rs 资产 | missing | NativeTaskBoard/Journal/BudgetLedger 未接入运行路径 |

### 1.4 agent_evolving(208 文件)→ 55.6%(done 3 / partial 4 / missing 2)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| evaluator | done | 本地判据 + LLM judge 附加 |
| trajectory | done | 抽取 + OTLP span codec + 聚合 |
| checkpointing/experience/sharing | done | 持久化 + 分享 + 评分(贝叶斯 E/U/F) |
| dataset | partial | Case/loader/split 有;curate 未全 |
| optimizer | partial | 规则梯度已做;LLM 梯度缺 |
| trainer | partial | 循环已做;prompt/tools 组件缺 |
| agent_rl | partial | reward + PPO step 已做;VERL/LoRA/gateway 缺 |
| updater | missing | multi_dim/single_dim 0% |
| signal | missing | from_conv/from_eval/skill_creation/team 0% |

### 1.5 rsi(180 文件)→ 72.2%(done 4 / partial 5 / missing 0)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| orchestrator | done | run_rounds 多轮 + checkpoint 续跑 |
| evaluator | done | expected 匹配 + evolving 轨迹评估 |
| member_optimizer | done | attribution→plan→execute→verify→publish 全链路 |
| single_harness | done | train/holdout + 候选门禁 + 续跑 |
| dataset_generator/curator/loader | partial | 确定性+LLM 合成已做;curate/分批缺 |
| evaluation_result_analyzer | partial | 确定性信号+归因已做;LLM 深度诊断缺 |
| team_skill_generator/optimizer | partial | 生成已做;演化缺 |
| auto_harness | partial | 六阶段+git/CI 已做;远端 PR/GitCode 缺 |
| storage/resource/config/schema | partial | in-memory |

### 1.6 extensions(102 文件)→ 55.0%(done 3 / partial 5 / missing 2)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| a2a | done | server/client + SSE 流式 |
| tracer_otel | done | JSONL + OTLP + semconv |
| mcp | done | stdio + http 客户端 |
| checkpointer Redis | partial | SET/GET/DEL/KEYS 已做;TTL/集群/pipeline 缺 |
| message_queue | partial | Redis 已做;Pulsar 缺 |
| store | partial | PG 已做;ES 缺 |
| external_provider OAuth | partial | 设备码已做;模型目录开发中(ah-plugins-model-catalog 未提交,2 测试失败) |
| context_evolver | partial | 任务记忆+检索+摘要已做;Milvus 缺 |
| sys_operation(远程沙箱) | missing | 9 provider 仅 4 白名单命令 |
| vendor_specific | missing | 重排/嵌入现词法 fallback |

### 1.7 dev_tools(94 文件)→ 80.0%(done 3 / partial 2 / missing 0)

| 子模块 | 状态 | 说明 |
| --- | --- | --- |
| agent_builder | done | NL→设计→DSL→执行 |
| tune | done | subagent 执行 + 评估/优化 + 最优跟踪 |
| symphony | done | 能力注册/指纹/检索/编排/执行 |
| skill_creator/evaluator | partial | 注册+评估已做;creator 脚本缺 |
| prompt_builder | partial | {{var}} 渲染;meta/feedback/badcase 缺 |

## 2. 加权总体

| 域 | 文件数 | 完成度 | 加权贡献 |
| --- | ---: | ---: | ---: |
| core | 796 | 52.6% | 418.7 |
| harness | 337 | 42.3% | 142.6 |
| agent_teams | 276 | 50.0% | 138.0 |
| agent_evolving | 208 | 55.6% | 115.6 |
| rsi | 180 | 72.2% | 130.0 |
| extensions | 102 | 55.0% | 56.1 |
| dev_tools | 94 | 80.0% | 75.2 |
| **合计** | **1993** | **≈ 54.0%** | **1076.2** |

## 3. 缺口 Top(按未完成面积)

1. `core/multi_agent`(31 文件,0%)——handoff/hierarchical/msgbus
2. `agent_teams` 20+ 辅助子模块(0%)——kv_cache/memory/models/rails/skill/prompts/cli/harness/mcp/spawn/worktree/reliability/observability/context/interaction 等
3. `harness` 工具长尾——browser/lsp/mobile/multimodal/worktree/cron/powershell/paid_search
4. `harness` rails 长尾——heartbeat/llm_retry/lsp/mcp/memory/security/skills/task_*/evolution/interrupt(约 15 条)
5. `agent_evolving` updater + signal(0%)
6. `extensions` 远程沙箱 9 provider + ES/Pulsar

## 4. 维护纪律

1. 每轮账目更新后同步本看板的状态列(与 REMAINING_PLAN.md 同源);
2. 状态变更必须附测试证据(文件:行),禁止只改看板不改代码;
3. partial 统一计 50%,如需精确,可在说明列标注子功能完成比例;
4. 本看板是估算,不作为验收依据;验收以 capability-map.md 的 done 状态 + 差分契约为准。