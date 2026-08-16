# 能力地图(capability-map.md)

> 目标:完整实现 agent-core(Python)全部功能。本文是唯一的能力↔seam↔插件↔工作包映射,
> 规划与验收都以它为准。状态标注:done / partial / missing / excluded,
> 依据代码证据(测试 + 真实路径),不是文档描述。

## 0. 源:agent-core(Python)域规模

| Python 域 | 文件数 | 顶层符号 | Rust 目标
| --- | ---: | ---: | --- |
| openjiuwen/core | 796 | 2226 | ah-plugins-core-* + ah-contracts 核心 seam
| openjiuwen/harness | 337 | 1613 | ah-plugins-harness-*(tools/rails/subagents/cli)
| openjiuwen/agent_teams | 276 | 1104 | ah-plugins-teams
| openjiuwen/agent_evolving | 208 | 602 | ah-plugins-evolving
| openjiuwen/rsi | 180 | 1695 | ah-plugins-rsi
| openjiuwen/extensions | 102 | 340 | ah-plugins-store/queue/transport/otel 等
| openjiuwen/dev_tools | 94 | 141 | ah-plugins-devtools
| openjiuwen/symphony | 0(仅 README) | 0 | ah-plugins-symphony(已落地:注册/指纹/检索/编排/执行)

## 1. Seam 清单(ah-contracts 目标)

| Seam | ServiceKey | Service Definition | 状态
| --- | --- | --- | --- |
| llm | `llm` | `ModelProvider`(已实现;`OpenAiConfig` 支持从 credentials seam 解析 key/base_url,credentials 优先、环境变量兜底) | done(契约)/ partial(openai-compatible + anthropic 真实协议已落地;其余 provider 未实现) |
| tools | `tools` | `Tool` + `ToolRegistry`(已实现) | done(契约)/ partial(已有真实工具,注册表通用) |
| prompt | `prompt` | `PromptRegistry`(已实现:版本化注册 + {{var}} 渲染 + 缺失变量显式报错 + 文件持久化);agent-loop 消费方注入渲染系统提示 | done(契约+消费)/ partial(无结构化 schema prompt) |
| store | `store` | `BaseKVStore`/`BaseMessageStore`(已实现,文件后端);DB/Vector 留待后续 | done(契约,本地文件+Redis+PostgreSQL 后端)/ partial(ES/Milvus) |
| session | `sessions` + `session-manager` | `SessionLog` append-only 日志 + JSONL 持久化 + 投影;`SessionManager` 多会话 create/open/fork/list(已实现) | done(契约)/ partial(无分布式/跨进程会话) |
| context | `context` | `ContextEngine`(已实现:预算组装/摘录压缩+LLM 总结/offload/reinject;agent-loop 与 subagent 消费) | done(契约+消费)/ partial(精确 tokenizer) |
| memory | `memory` | `MemoryProvider`(已实现:JSON 文件持久化 + remember/recall/forget 工具);graph-memory 知识图谱记忆(实体/关系/episode + 检索/邻居) | done(契约+图记忆)/ partial(外部 provider) |
| retrieval | `retrieval` | `RetrievalProvider` + `Reranker`(已实现:BM25 + 本地确定性向量(哈希 n-gram TF + 余弦) + JSON 持久化 + ingest/search 工具 + 词法/向量融合重排(多样性惩罚)) | done(契约)/ partial(无外部模型 embedding) |
| fs | `fs` | `FsProvider`(已实现,真实本地) | done(契约)/ partial(仅本地) |
| shell | `shell` | `ShellProvider`(已实现,真实本地) | done(契约)/ partial(仅本地) |
| code | `code` | `CodeProvider`(已实现:隔离 scratch + python3 子进程 + 超时强杀 + 输出/退出码) | done(契约)/ partial(仅 python3,无沙箱容器) |
| sandbox | `sandbox` | `SandboxProvider`(已实现:策略化,sandbox.json 允许前缀/拒绝命令模式/绝对路径开关 + pre-execute rail 消费) | done(契约+消费,本地)/ partial(远程沙箱容器/VM) |
| security | `security` | `SecurityProvider`(已实现:规则 guardrails + pre-execute rail) | done(契约)/ partial(无 LLM 后端/API) |
| agent-loop | `agent-loop` | `AgentLoop`(已实现,真实 ReAct,日志驱动,工具错误回喂模型) | done
| workflow | `workflow` | `WorkflowEngine`(已实现:Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + 条件边 + 轨迹入日志;LLM 节点流式消费) | done(契约+流式消费) |
| subagent | `subagent` | `SubagentRuntime`(已实现:隔离会话委派 + 预算 + 上下文注入 + delegate_task 工具) | done(契约)/ partial(无进程外/跨产品子代理) |
| teams | `teams` | `TeamRuntime`(已实现:内存 + SQLite 持久化两套运行时:任务板/依赖门控/成员校验/review/settle/run_task 真实委派/teams/task 事件/消息经 queue seam 传输) | done(契约+持久化+消息)/ partial(外部 CLI 进程/ZMQ) |
| evolving | `evolving` | `EvolvingRuntime`(已实现:轨迹从会话日志真实抽取;本地判据评估 + LLM judge 附加;优化建议规则推导 + LLM 附加) | done(契约)/ partial(无持久化/RL) |
| rsi | `rsi` | `RsiRuntime`(已实现:数据集生成 + 用例真实执行 + evolving 评估 + 报告 + 提示精化 + checkpoint 落盘) | done(契约)/ partial(无 LLM 数据生成/RL) |
| telemetry | `telemetry` | `TelemetryProvider`(已实现:内存 span 记录 + JSONL 文件导出,挂 agent/step 与 tools/post-execute 监听生成真实 span) | done(契约+JSONL+OTLP/JSON 导出+semconv 语义约定) |
| queue | `queue` | `MessageQueue`(已实现:文件后端,每 channel append-only JSONL + 消费游标 offset 语义,重启恢复;teams 消息消费方) | done(契约+消费,本地+Redis 外部后端)/ partial(外部 Pulsar/ZMQ) |
| mcp | `mcp` | `McpClient`(已实现:真实 stdio 子进程 + newline-delimited JSON-RPC 2.0,握手/list_tools/call_tool/shutdown) | done(契约+stdio+http 客户端) |
| transport | `transport` | A2A 风格传输(JSON-RPC 2.0 + SSE 流式 over HTTP;ureq 客户端 + 本地 HTTP/1.1 服务端) | done(契约+客户端+服务端+流式)/ partial(加密传输) |
| credentials | `credentials` | `CredentialProvider`(已实现:真实环境变量 provider,映射可配置,如 `openai.api_key` → `OPENAI_API_KEY`;`get`/`list` 真实读 `std::env`,`set`/`remove` 显式报错 env 只读) | done(契约)/ partial(env 只读,无密钥管理后端;ah-plugins-credentials:tests 覆盖 get/list/set/remove 真实 env 路径,openai 集成测试经 credentials 解析 key 后真实 HTTP 往返) |

## 2. 域 → seam/插件映射(目标态)

### 2.1 core(2226 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| common(日志/错误/客户端注册表) | ah-contracts 类型 + ah-plugins-core-common | 错误模型、日志事件、client registry、后台任务 | 错误语义对等;日志事件结构对等
| foundation/llm(11 个 provider) | `llm` seam + ah-plugins-openai/anthropic/dashscope/deepseek 等 | 模型调用、流式、工具调用组装、provider 错误 | openai-compatible + anthropic 已落地(Messages API system 顶层/tool_use·tool_result 块/x-api-key+version 头,本地 HTTP 协议往返);流式已落地(本回合:stream_chat SSE 增量,content/tool_calls delta 累加 + [DONE]);dashscope/deepseek 留待后续 |
| foundation/tool | `tools` seam | tool 元数据、auth、调用、流式 chunk、校验 | 工具调用链真实执行
| foundation/prompt | `prompt` seam | 模板渲染、结构化 prompt | 确定性渲染({{var}} + 版本化文件后端已落地) |
| foundation/store(kv/db/vector/message/graph/object) | `store` seam + ah-plugins-store(本地文件后端);redis/gaussdb/elasticsearch/milvus/chroma 待后续 | 持久化(kv/message 已真实落盘) | 本地后端已测;外部后端集成测试留待后续 |
| application(llm_agent/workflow_agent) | ah-plugins-core-application | 绑定配置/工作流/记忆/会话 | 真实模型循环
| workflow(78 类) | ah-plugins-workflow-engine | 组件、分支、循环、子工作流、检查点、流式 | Http/Intent/Questioner + 检查点续跑已落地;llm 流式 seam + 工作流 LLM 节点流式消费已落地(本回合:run_llm 经 stream_chat 消费,SSE 增量累加 + 工具调用组装);Pregel 见下 |
| graph/Pregel(53 类) | ah-plugins-pregel | 状态通道、中断、动态路由 | 已落地(本回合:超级步引擎 + missing/present/equals 条件 + halt 中断 + 上限) |
| controller(57 类) | `controller` seam + ah-plugins-controller | 任务调度/执行器/意图识别 | 已落地(本回合:任务 CRUD/状态机/优先级/父子层级防环;执行器注册表 + 同会话冲突拒绝;确定性意图识别;LLM 意图留待后续) |
| operator(8 类) | `operator` seam + ah-plugins-operator | LLM/Tool/Memory/Skill 算子 | 已落地(本回合:自进化参数句柄,LLM/tool/memory/skill 四算子 + freeze 检查 + 回调同步 + 检查点) |
| runner(46 类) | `runner` seam + ah-plugins-runner | 回调链、资源管理、取消、超时 | 已落地(本回合:优先级降序执行 + retry/timeout/break/rollback 逆序回滚 + CallbackMetrics;资源管理/取消留待后续) |
| session(38 类) | `sessions` seam + ah-plugins-session-log | 检查点、VCS/fork/restore、tracer | append-only 日志重建 + fork/checkpoint/restore 已落地(本回合) |
| context_engine(72 文件) | `context` seam + `tokenizer` seam | 压缩、offload、token 预算、reinjection | 已实现(ah-plugins-context:预算组装/摘录压缩+LLM 总结/offload JSONL/reinject;精确 tokenizer 已落地(BPE-lite + CJK 感知,注册后 estimate_tokens 用精确计数));向量化留待后续 |
| memory(104 文件) | `memory` seam + ah-plugins-graph-memory | graph/lite/coding 记忆、外部 provider | 图记忆已落地(本回合:确定性实体抽取 + 共现关系 + episode + JSONL 持久化 + 关键词检索/邻居遍历 + graph_* 工具);lite/coding 与外部 provider 留待后续 |
| retrieval(84 文件) | `retrieval` seam | indexing/embedding/reranker/vector store/retriever | BM25 + 本地确定性向量(哈希 n-gram TF + 余弦)已落地;reranker 本地确定性融合重排已落地(ah-plugins-rerank:词法+向量归一化加权融合 + 多样性惩罚);外部模型 embedding 留待后续 |
| security(20 类) | `security` seam | guardrail 后端、sanitizer、风险组合 | 规则+LLM 后端
| sys_operation(56 类) | `fs`/`shell`/`code`/`sandbox` seam + ah-plugins-sysop-* | 本地/远程受限执行 | 现成 sys_operation 资产;补远程沙箱
| single_agent(60 类) | ah-plugins-core-single-agent | ReAct、中断恢复、skills、ability manager | 中断可恢复;非 mock 模型
| multi_agent(26 类) | 见 agent_teams 域 | handoff/hierarchical/msgbus | 见 2.3

### 2.2 harness(1613 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| tools(638 符号/117 文件) | `tools` seam + ah-plugins-harness-tools | 浏览器/web/LSP/shell/fs/mcp/多模态/移动 GUI | web_fetch / run_code 真实工具已注册(agent 可调用);browser/lsp 需外部进程 |
| rails(143/57 文件) | 事件监听器(waterfall) | 规划/完成/心跳/重试/LSP/MCP/渐进工具 等 | ShellGuard + PathGuard + ToolBudget + 渐进披露 ApprovalRail(本回合,tool-approval seam,批准集持久化)已落地;其余按需补充 |
| subagents(24/8 文件) | `subagents` seam + ah-plugins-subagents | code/research/plan/verify + browser/mobile | code/research/plan/verify 已落地(本回合:类型提示 + 工具白名单真实强制);browser/mobile 留待后续 |
| cli(113/19 文件) | ah-plugins-cli | REPL、会话存储、渲染 | 已落地(本回合:Claude Code 风格渲染器 ● Tool(args)/⎿ 摘要/☑☐ todo checkbox/⚙ 消息,事件→块投影,CLI 实时渲染;REPL+会话存储先前已落地)
| workspace/goal/manifest(本回合已落地:workspace.json + 目标状态机)/ resources/schema/security/prompts/kv_cache/lsp | 各插件 | 对应功能 | 逐模块对等;workspace 已实现 |

### 2.3 agent_teams(1104 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| schema(81) | ah-plugins-teams | TeamAgentSpec/DeepAgentSpec/事件体系 | 字段对等
| agent/coordination/scheduling(57) | `teams` seam + teams-workflow | 协调内核、调度、生命周期 | swarmflow 编排已落地;生命周期/调度细化留待后续 |
| runtime(27) | `teams` seam | 任务板/依赖/review/settle 真实状态迁移,run_task 真实委派 | 已落地(ah-plugins-teams);持久化/池/7 路 dispatch 留待后续 |
| messager(16) | `queue` seam + ah-plugins-queue(本地日志+游标);ZMQ ROUTER/DEALER 留待后续 | 本地队列已落地(本回合) |
| external(95) | `external` seam + ah-plugins-external | 外部 CLI agent、SSH | 已落地(本回合:真实子进程运行时,流式 stdin + 单发 argv 两风味,adapter 启动知识/完成标记/steer/abort;SSH 留待后续) |
| workflow(148) | ah-plugins-teams-workflow | swarmflow 引擎(phase/agent 并行 barrier/预算/事件流/journal 续跑) | 已落地(本回合,SwarmflowEngine,worker=SubagentRuntime) |
| residual.rs 资产 | ah-plugins-teams | NativeTaskBoard/Journal/BudgetLedger/检测器 | 接入运行路径(现仅测试引用)
| kv_cache/memory/monitor/models/rails/skill/prompts/cli/harness | 各插件 | 对应功能 | monitor 已落地(本回合:ah-plugins-team-monitor 只读团队/任务/消息视图 + teams/task 事件日志);kv_cache/memory/models 等留待后续 |

### 2.4 agent_evolving(602 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| dataset(5) | ah-plugins-evolving | Case/EvaluatedCase/loader/shuffle/split | 字段语义对等
| evaluator(40) | `evolving` seam | LLM-as-judge、指标、pipeline | 已落地(本地确定性判据必算 + LLM judge 附加,不可用原因显式记录,不静默) |
| trajectory(94) | `evolving` seam | OTLP span codec、抽取、聚合、存储 | 已落地(本回合:Trajectory↔Span 树编解码 + 聚合统计,抽取/经验存储先前已落地) |
| checkpointing/experience/sharing(21/54/20) | ah-plugins-evolving | 持久化、评分、分享 | experience 持久化已落地(本回合:save/load/search JSONL + 跨重开恢复);评分/分享留待后续 |
| optimizer/updater/signal(59/3/32) | `optimizer` seam + ah-plugins-optimizer | LLM 梯度优化、信号检测 | 已落地(本回合:文本梯度 backward(失败信号过滤 + 问题→参数路由)+ step 经 OperatorRegistry 应用(冻结/缺失显式记录));LLM 梯度与信号检测留待后续 |
| agent_rl(216) | ah-plugins-rl + `rl-step` seam + ah-plugins-rl-step | VERL/PPO、reward、LoRA、gateway | reward 已落地;训练步数学已落地(本回合:advantage(reward−value)/policy ratio/clipped PPO objective/value loss + 聚合,无效/非有限样本剔除);VERL 训练器/LoRA/gateway 留待后续 |
| trainer/prompts/tools(3/18/23) | `trainer` seam + ah-plugins-trainer | 训练循环、prompt、工具 | 训练循环已落地(本回合:基线评估 → 每轮 train 前向 → Optimizer 文本梯度应用(经 OperatorRegistry)→ 验证门禁 → 改进推进 best → early stop);prompt/tools 组件留待后续 |

### 2.5 rsi(1695 符号) + auto_harness

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| orchestrator(100) | ah-plugins-rsi | 多轮优化编排、checkpoint/resume | 已落地:run_rounds 多轮循环(评测→精化→checkpoint 续跑,本回合);git/CI 基建留待后续 |
| dataset_generator(72) / dataset_curator(13) / data_loader(5) | ah-plugins-rsi | LLM 生成、curate、分批 | 已落地(确定性扩展 + LLM 合成:提示→JSON 任务变体解析/去重,LLM 不可用显式回退确定性扩展);curate/分批留待后续 |
| evaluator(judger 91/case_runner 56/…) | `evolving` seam + ah-plugins-rsi | LLM judge、执行后端 | 已落地(expected 匹配优先 + evolving 轨迹评估(本地判据 + LLM judge 附加)) |
| evaluation_result_analyzer(73) | ah-plugins-rsi | 信号提取、根因归因、证据引用、artifact 落盘 | 已落地(本回合:确定性信号 + 规则归因 + analysis.json);LLM 深度诊断留待后续 |
| member_optimizer(16 文件) | `member-optimizer` seam + ah-plugins-member-optimizer | attribution→plan→execute→verify→publish | 已落地(本回合:确定性机制归因(prompt/tool/skill/memory/workflow/context→lever/目标面)+ 计划(文本梯度)+ 经 Optimizer/OperatorRegistry 执行 + val 集验证门禁 + best 引用 JSON 落盘) |
| team_skill_generator/optimizer(26/8) | `team-skill` seam + ah-plugins-team-skill | 技能生成与演化 | 生成已落地(本回合:任务 → 确定性计划(关键词→能力/步骤)+ 注册 skill seam + 源任务验证(子代理+evolving)+ 修复重试);演化留待后续 |
| single_harness(72+12) | `single-harness` seam + ah-plugins-rsi-single-harness | 迭代编排、候选门禁 | 已落地(本回合:train/holdout 拆分 + 每 epoch 评测→精化→候选 holdout 门禁(严格优于才接受)+ best/checkpoint JSONL 落盘 + 中断续跑) |
| auto_harness(65 文件) | ah-plugins-autoharness | assess/plan/implement/verify/commit/publish + 真实 git/CI | 编排已落地(本回合:六阶段真实执行,git 提交+分支);远端 PR/GitCode 留待后续 |
| storage/resource/config/schema | ah-plugins-rsi | 持久化、资源记账、配置、类型 | 现 in-memory

### 2.6 extensions(340 符号) + providers

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| checkpointer(Redis) | `store` seam + ah-plugins-store-redis | TTL/集群/pipeline | Redis 后端已落地(本回合:真实 SET/GET/DEL/KEYS,与文件后端同 seam 互换) |
| message_queue(Pulsar) | `queue` seam + ah-plugins-queue-redis | producer/consumer/replay | 已落地(本回合:Redis LIST 日志 + INCR 序号 + 游标,真实外部 provider 可互换后端;Pulsar 留待后续) |
| store(GaussDB/ES) | `store` seam + ah-plugins-store-pg(GaussDB 兼容 SQL)/elasticsearch | SQL/向量检索 | 已落地(本回合:真实 PostgreSQL SQL 后端,kv/messages 两表 + UPSERT + 增量读,与文件/Redis 同 seam 互换;ES 留待后续) |
| sys_operation(远程沙箱 9 provider) | `sandbox` seam + 进程插件 | AIO/jiuwenbox/yuanrong | 现 4 个白名单命令
| external_provider(OpenAI OAuth) | `oauth` seam + ah-plugins-oauth | 设备码 OAuth、模型目录 | 设备码流已落地(本回合:start/poll,pending/expired 映射);模型目录留待后续 |
| a2a | `transport` seam + ah-plugins-transport | HTTP server/client、流式 | server/client + SSE 流式已落地(本回合:stream_send 与 text/event-stream 端点) |
| tracer_otel | `telemetry` seam + ah-plugins-telemetry | span 记录 + JSONL 导出 + OTLP/JSON + 语义约定 | JSONL 真实;OTLP/JSON 导出已落地;semconv 语义约定已落地(本回合:semconv 模块 1:1 对齐 tracer_otel.semconv — gen_ai.*/openjiuwen.workflow.*/openjiuwen.agent.*/openjiuwen.* 全量常量 + agent/base 属性构建助手;agent step 与 tool span 携带 semconv 属性) |
| context_evolver(58 文件) | `memory-evolver` seam + ah-plugins-context-evolver | LLM 记忆流水线、Milvus | 已落地(本回合:任务记忆保存(JSONL)/关键词+标签检索/轨迹凝练摘要/上下文注入;Milvus 向量后端留待后续) |
| mcp(stdio/http) | `mcp` seam + ah-plugins-mcp | stdio 子进程、newline-delimited JSON-RPC 2.0、initialize 握手、list_tools/call_tool/shutdown | stdio 真实 + http 客户端真实(本回合:McpHttpClient POST JSON-RPC,本地 HTTP 端点往返验证) |
| vendor_specific | `llm` seam | 各厂商重排/嵌入 | 现词法 fallback

### 2.7 dev_tools(141 符号) + symphony

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| prompt_builder | ah-plugins-devtools | meta/feedback/badcase 构建 | 现 {{var}} 渲染
| agent_builder | ah-plugins-agentbuilder | NL→设计→DSL→执行 | 已落地(本回合:确定性意图解析 → WorkflowSpec DSL → WorkflowEngine 真实执行) |
| tune | ah-plugins-tune | optimizer/evaluator/trainer 流水线 | 已落地(本回合:subagent 执行 + evolving 评估/优化精化 prompt + 最优跟踪) |
| skill_creator/evaluator | ah-plugins-skill | 技能注册/持久化/评估 | 已落地(本回合:文件后端 + subagent 委派 + evolving 轨迹评估) |
| symphony | ah-plugins-symphony | 能力检索/编排/legacy runtime | 已落地(本回合:能力注册 + 语义指纹 + 任务检索 + 可解释计划 + 工具/subagent 真实执行,JSONL 持久化)

## 3. 工作包生命周期与验收标准

每个能力按以下生命周期推进(详见 development.md):

1. **契约**(contracts):seam trait + 纯类型 + 单元测试;
2. **Mock**(plugin-mock):可 boot 的确定性实现,保证系统可运行;
3. **真实**(plugin-prod):真实协议/持久化/子进程实现;
4. **对等**(parity):差分契约 + golden fixtures + e2e,证明与 agent-core 行为对等。

验收铁律:**本地/mock 测试通过 ≠ 完成**。done 状态要求:生产路径真实执行,
无 unsupported/fallback/mock 替代,并有测试证据(文件:行)。