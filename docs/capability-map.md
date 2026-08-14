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
| openjiuwen/symphony | 0(仅 README) | 0 | ah-plugins-symphony(新实现)

## 1. Seam 清单(ah-contracts 目标)

| Seam | ServiceKey | Service Definition | 状态
| --- | --- | --- | --- |
| llm | `llm` | `ModelProvider`(已实现;`OpenAiConfig` 支持从 credentials seam 解析 key/base_url,credentials 优先、环境变量兜底) | done(契约)/ partial(真实 provider + credentials 集成 e2e 已落地,其余 provider 未实现) |
| tools | `tools` | `Tool` + `ToolRegistry`(已实现) | done(契约)/ partial(已有真实工具,注册表通用) |
| prompt | `prompt` | `PromptRegistry`(已实现:版本化注册 + {{var}} 渲染 + 缺失变量显式报错 + 文件持久化);agent-loop 消费方注入渲染系统提示 | done(契约+消费)/ partial(无结构化 schema prompt) |
| store | `store` | `BaseKVStore`/`BaseMessageStore`(已实现,文件后端);DB/Vector 留待后续 | done(契约,本地文件后端)/ partial(外部后端 Redis/GaussDB/ES/Milvus) |
| session | `sessions` + `session-manager` | `SessionLog` append-only 日志 + JSONL 持久化 + 投影;`SessionManager` 多会话 create/open/fork/list(已实现) | done(契约)/ partial(无分布式/跨进程会话) |
| context | `context` | `ContextEngine`(已实现:预算组装/摘录压缩+LLM 总结/offload/reinject;agent-loop 与 subagent 消费) | done(契约+消费)/ partial(精确 tokenizer) |
| memory | `memory` | `MemoryProvider`(已实现:JSON 文件持久化 + remember/recall/forget 工具) | done(契约)/ partial(无图记忆/外部 provider) |
| retrieval | `retrieval` | `RetrievalProvider`(已实现:BM25 + 本地确定性向量(哈希 n-gram TF + 余弦) + JSON 持久化 + ingest/search 工具) | done(契约)/ partial(无外部模型 embedding/reranker) |
| fs | `fs` | `FsProvider`(已实现,真实本地) | done(契约)/ partial(仅本地) |
| shell | `shell` | `ShellProvider`(已实现,真实本地) | done(契约)/ partial(仅本地) |
| code | `code` | `CodeProvider`(已实现:隔离 scratch + python3 子进程 + 超时强杀 + 输出/退出码) | done(契约)/ partial(仅 python3,无沙箱容器) |
| sandbox | `sandbox` | `SandboxProvider`(已实现:策略化,sandbox.json 允许前缀/拒绝命令模式/绝对路径开关 + pre-execute rail 消费) | done(契约+消费,本地)/ partial(远程沙箱容器/VM) |
| security | `security` | `SecurityProvider`(已实现:规则 guardrails + pre-execute rail) | done(契约)/ partial(无 LLM 后端/API) |
| agent-loop | `agent-loop` | `AgentLoop`(已实现,真实 ReAct,日志驱动,工具错误回喂模型) | done
| workflow | `workflow` | `WorkflowEngine`(已实现:Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + 条件边 + 轨迹入日志) | done(契约)/ partial(无流式) |
| subagent | `subagent` | `SubagentRuntime`(已实现:隔离会话委派 + 预算 + 上下文注入 + delegate_task 工具) | done(契约)/ partial(无进程外/跨产品子代理) |
| teams | `teams` | `TeamRuntime`(已实现:内存 + SQLite 持久化两套运行时:任务板/依赖门控/成员校验/review/settle/run_task 真实委派/teams/task 事件/消息经 queue seam 传输) | done(契约+持久化+消息)/ partial(外部 CLI 进程/ZMQ) |
| evolving | `evolving` | `EvolvingRuntime`(已实现:轨迹从会话日志真实抽取;本地判据评估 + LLM judge 附加;优化建议规则推导 + LLM 附加) | done(契约)/ partial(无持久化/RL) |
| rsi | `rsi` | `RsiRuntime`(已实现:数据集生成 + 用例真实执行 + evolving 评估 + 报告 + 提示精化 + checkpoint 落盘) | done(契约)/ partial(无 LLM 数据生成/RL) |
| telemetry | `telemetry` | `TelemetryProvider`(已实现:内存 span 记录 + JSONL 文件导出,挂 agent/step 与 tools/post-execute 监听生成真实 span) | done(契约)/ partial(JSONL 导出真实;OTLP 导出留待后续)
| queue | `queue` | `MessageQueue`(已实现:文件后端,每 channel append-only JSONL + 消费游标 offset 语义,重启恢复;teams 消息消费方) | done(契约+消费,本地)/ partial(外部 Pulsar/ZMQ) |
| mcp | `mcp` | `McpClient`(已实现:真实 stdio 子进程 + newline-delimited JSON-RPC 2.0,握手/list_tools/call_tool/shutdown) | done(契约)/ partial(stdio 真实,http 未实现) |
| transport | `transport` | A2A 风格传输(已实现:JSON-RPC 2.0 over HTTP,agent/getCard + message/send;ureq 客户端 + 本地 HTTP/1.1 服务端) | done(契约+客户端+本地服务端)/ partial(SSE/流式/加密传输) |
| credentials | `credentials` | `CredentialProvider`(已实现:真实环境变量 provider,映射可配置,如 `openai.api_key` → `OPENAI_API_KEY`;`get`/`list` 真实读 `std::env`,`set`/`remove` 显式报错 env 只读) | done(契约)/ partial(env 只读,无密钥管理后端;ah-plugins-credentials:tests 覆盖 get/list/set/remove 真实 env 路径,openai 集成测试经 credentials 解析 key 后真实 HTTP 往返) |

## 2. 域 → seam/插件映射(目标态)

### 2.1 core(2226 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| common(日志/错误/客户端注册表) | ah-contracts 类型 + ah-plugins-core-common | 错误模型、日志事件、client registry、后台任务 | 错误语义对等;日志事件结构对等
| foundation/llm(11 个 provider) | `llm` seam + ah-plugins-openai/anthropic/dashscope/deepseek 等 | 模型调用、流式、工具调用组装、provider 错误 | 每个 provider 真实协议路径 + 差分契约
| foundation/tool | `tools` seam | tool 元数据、auth、调用、流式 chunk、校验 | 工具调用链真实执行
| foundation/prompt | `prompt` seam | 模板渲染、结构化 prompt | 确定性渲染({{var}} + 版本化文件后端已落地) |
| foundation/store(kv/db/vector/message/graph/object) | `store` seam + ah-plugins-store(本地文件后端);redis/gaussdb/elasticsearch/milvus/chroma 待后续 | 持久化(kv/message 已真实落盘) | 本地后端已测;外部后端集成测试留待后续 |
| application(llm_agent/workflow_agent) | ah-plugins-core-application | 绑定配置/工作流/记忆/会话 | 真实模型循环
| workflow(78 类) | ah-plugins-workflow-engine | 组件、分支、循环、子工作流、检查点、流式 | 现成 rp301 资产;补 Http/Questioner/Intent 真实语义
| graph/Pregel(53 类) | 同上 | 状态通道、中断、动态路由 | Pregel 语义完整
| controller(57 类) | 同上 | 任务调度/执行器/意图识别 | 真实调度与冲突处理
| operator(8 类) | 同上 | LLM/Tool/Memory/Skill 算子 | 算子真实调用(现为回显)
| runner(46 类) | ah-plugins-core-runner | 回调链、资源管理、取消、超时 | 回调链对等
| session(38 类) | `sessions` seam + ah-plugins-session-log | 检查点、VCS/fork/restore、tracer | append-only 日志重建 + fork/checkpoint/restore 已落地(本回合) |
| context_engine(72 文件) | `context` seam | 压缩、offload、token 预算、reinjection | 已实现(ah-plugins-context:预算组装/摘录压缩+LLM 总结/offload JSONL/reinject);精确 tokenizer 与向量化留待后续 |
| memory(104 文件) | `memory` seam + ah-plugins-memory-* | graph/lite/coding 记忆、外部 provider | 记忆持久化 + provider 真实接入
| retrieval(84 文件) | `retrieval` seam | indexing/embedding/reranker/vector store/retriever | BM25 + 本地确定性向量(哈希 n-gram TF + 余弦)已落地;reranker 与外部模型 embedding 留待后续 |
| security(20 类) | `security` seam | guardrail 后端、sanitizer、风险组合 | 规则+LLM 后端
| sys_operation(56 类) | `fs`/`shell`/`code`/`sandbox` seam + ah-plugins-sysop-* | 本地/远程受限执行 | 现成 sys_operation 资产;补远程沙箱
| single_agent(60 类) | ah-plugins-core-single-agent | ReAct、中断恢复、skills、ability manager | 中断可恢复;非 mock 模型
| multi_agent(26 类) | 见 agent_teams 域 | handoff/hierarchical/msgbus | 见 2.3

### 2.2 harness(1613 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| tools(638 符号/117 文件) | `tools` seam + ah-plugins-harness-tools | 浏览器/web/LSP/shell/fs/mcp/多模态/移动 GUI | web 真实适配器已落地(ah-plugins-web,ureq);browser/lsp 需外部进程 |
| rails(143/57 文件) | 事件监听器(waterfall) | 规划/完成/心跳/重试/LSP/MCP/渐进工具 等 | 挂载点已就绪(tools/pre-execute);ShellGuard + PathGuard + ToolBudget(ah-plugins-rails)已落地;其余 rail 按需补充 |
| subagents(24/8 文件) | `subagents` seam | code/research/plan/browser/mobile/verify | 真实子代理运行时(现为 Unsupported/Mock)
| cli(113/19 文件) | ah-plugins-harness-cli | REPL、会话存储、渲染 | 现为 CliUiAdapter::unsupported
| workspace/goal/manifest(本回合已落地:workspace.json + 目标状态机)/ resources/schema/security/prompts/kv_cache/lsp | 各插件 | 对应功能 | 逐模块对等;workspace 已实现 |

### 2.3 agent_teams(1104 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| schema(81) | ah-plugins-teams | TeamAgentSpec/DeepAgentSpec/事件体系 | 字段对等
| agent/coordination/scheduling(57) | `teams` seam | 协调内核、调度、生命周期 | 现为 local-distributed-mock
| runtime(27) | `teams` seam | 任务板/依赖/review/settle 真实状态迁移,run_task 真实委派 | 已落地(ah-plugins-teams);持久化/池/7 路 dispatch 留待后续 |
| messager(16) | `queue` seam + ah-plugins-queue(本地日志+游标);ZMQ ROUTER/DEALER 留待后续 | 本地队列已落地(本回合) |
| external(95) | `subagents` seam + 进程插件 | 外部 CLI agent、SSH | 全 crate 现无 std::process
| workflow(148) | ah-plugins-teams-workflow | swarmflow 引擎 | 现为 MockWorkflowStep
| residual.rs 资产 | ah-plugins-teams | NativeTaskBoard/Journal/BudgetLedger/检测器 | 接入运行路径(现仅测试引用)
| kv_cache/memory/monitor/models/rails/skill/prompts/cli/harness | 各插件 | 对应功能 | 现 0%

### 2.4 agent_evolving(602 符号)

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| dataset(5) | ah-plugins-evolving | Case/EvaluatedCase/loader/shuffle/split | 字段语义对等
| evaluator(40) | `evolving` seam | LLM-as-judge、指标、pipeline | 现为 mock 精确匹配
| trajectory(94) | `evolving` seam | OTLP span codec、抽取、聚合、存储 | 现为纯数据结构
| checkpointing/experience/sharing(21/54/20) | ah-plugins-evolving | 持久化、评分、分享 | 现为内存
| optimizer/updater/signal(59/3/32) | `evolving` seam | LLM 梯度优化、信号检测 | 现为确定性假更新
| agent_rl(216) | ah-plugins-rl | VERL/PPO、reward、LoRA、gateway | 现为合成 reward
| trainer/prompts/tools(3/18/23) | ah-plugins-evolving | 训练循环、prompt、工具 | 现 0%

### 2.5 rsi(1695 符号) + auto_harness

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| orchestrator(100) | ah-plugins-rsi | 多轮优化编排、checkpoint/resume | 已落地:run_rounds 多轮循环(评测→精化→checkpoint 续跑,本回合);git/CI 基建留待后续 |
| dataset_generator(72) / dataset_curator(13) / data_loader(5) | ah-plugins-rsi | LLM 生成、curate、分批 | 已落地确定性扩展(改写/组合/边界);LLM 生成留待后续 |
| evaluator(judger 91/case_runner 56/…) | `evolving` seam + ah-plugins-rsi | LLM judge、执行后端 | 现为字符串相等
| evaluation_result_analyzer(73) | ah-plugins-rsi | LLM 诊断、证据冲突修复 | 现 0%
| member_optimizer(16 文件) | ah-plugins-rsi | attribution→plan→execute→verify→publish | 现为启发式 plan
| team_skill_generator/optimizer(26/8) | ah-plugins-rsi | 技能生成与演化 | 现 0%/启发式
| single_harness(72+12) | ah-plugins-rsi | 迭代编排、候选门禁 | 现 0%
| auto_harness(65 文件) | ah-plugins-auto-harness | assess/plan/implement/verify/commit/publish + 真实 git/CI | 现 MockGit/MockCi
| storage/resource/config/schema | ah-plugins-rsi | 持久化、资源记账、配置、类型 | 现 in-memory

### 2.6 extensions(340 符号) + providers

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| checkpointer(Redis) | `store` seam + ah-plugins-redis | TTL/集群/pipeline | 现本地 fallback
| message_queue(Pulsar) | `queue` seam + ah-plugins-pulsar | producer/consumer/replay | 现 broadcast 兜底
| store(GaussDB/ES) | `store` seam + ah-plugins-gaussdb/elasticsearch | SQL/向量检索 | 现执行日志/内存余弦
| sys_operation(远程沙箱 9 provider) | `sandbox` seam + 进程插件 | AIO/jiuwenbox/yuanrong | 现 4 个白名单命令
| external_provider(OpenAI OAuth) | `credentials` seam + ah-plugins-openai-auth | 设备码 OAuth、模型目录 | 现恒 unsupported
| a2a | `transport` seam + ah-plugins-a2a | HTTP server/client、流式 | 现 local adapter
| tracer_otel | `telemetry` seam + ah-plugins-telemetry | span 记录 + JSONL 导出(已落地);OTLP 导出、semconv 留待后续 | JSONL 真实(ah-plugins-telemetry:tests 覆盖 record/export/事件监听);OTLP 规划 |
| context_evolver(58 文件) | `memory` seam + ah-plugins-context-evolver | LLM 记忆流水线、Milvus | 现 0%
| mcp(stdio/http) | `mcp` seam + ah-plugins-mcp | stdio 子进程、newline-delimited JSON-RPC 2.0、initialize 握手、list_tools/call_tool/shutdown | stdio 真实(ah-plugins-mcp:tests/mcp_stdio.rs 真实子进程验证);http 规划 |
| vendor_specific | `llm` seam | 各厂商重排/嵌入 | 现词法 fallback

### 2.7 dev_tools(141 符号) + symphony

| Python 子模块 | seam / 插件 | 关键功能 | 验收要点
| --- | --- | --- | --- |
| prompt_builder | ah-plugins-devtools | meta/feedback/badcase 构建 | 现 {{var}} 渲染
| agent_builder | ah-plugins-devtools | NL→设计→DL→DSL→执行 | 现 runtime=mock manifest
| tune | ah-plugins-devtools | optimizer/evaluator/trainer 流水线 | 现 MockEvaluator/MockTrainer
| skill_creator/evaluator | ah-plugins-devtools | 技能生成/评估 | 现格式启发式
| symphony | ah-plugins-symphony | 能力检索/编排/legacy runtime | Python 侧无代码,按 README 语义实现

## 3. 工作包生命周期与验收标准

每个能力按以下生命周期推进(详见 development.md):

1. **契约**(contracts):seam trait + 纯类型 + 单元测试;
2. **Mock**(plugin-mock):可 boot 的确定性实现,保证系统可运行;
3. **真实**(plugin-prod):真实协议/持久化/子进程实现;
4. **对等**(parity):差分契约 + golden fixtures + e2e,证明与 agent-core 行为对等。

验收铁律:**本地/mock 测试通过 ≠ 完成**。done 状态要求:生产路径真实执行,
无 unsupported/fallback/mock 替代,并有测试证据(文件:行)。
